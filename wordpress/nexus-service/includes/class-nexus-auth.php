<?php
if (!defined('ABSPATH')) { exit; }
final class Nexus_Auth {
    public static function admin_ids(): array {
        return array_values(array_filter(array_map('trim', explode(',',Nexus_Store::setting('admin_github_ids'))), fn($id)=>ctype_digit($id) && (int)$id>0));
    }
    public static function account(array $user): array {
        return ['github_id'=>(int)$user['github_id'],'login'=>$user['login'],'admin'=>in_array((string)$user['github_id'],self::admin_ids(),true)];
    }
    public static function caller($request) {
        $header=$request->get_header('authorization');
        if (!$header) { return null; }
        if (!preg_match('/^Bearer ([a-f0-9]{64})$/D',$header,$match)) { return new WP_Error('unauthorized','Please sign in again.',['status'=>401]); }
        $session=Nexus_Store::get('session',hash('sha256',$match[1]));
        return $session ? self::account($session) : new WP_Error('unauthorized','Please sign in again.',['status'=>401]);
    }
    public static function permission($request) {
        $caller=self::caller($request);
        if (is_wp_error($caller)) { return $caller; }
        if (!$caller) { return new WP_Error('unauthorized','Sign in required.',['status'=>401]); }
        if (str_contains($request->get_route(),'/admin/') && !$caller['admin']) {
            return new WP_Error('forbidden','Administrator access required.',['status'=>403]);
        }
        return true;
    }
    private static function oauth(string $path,array $body) {
        $response=wp_remote_post('https://github.com/'.$path,['headers'=>['Accept'=>'application/json','User-Agent'=>'Straconis-Nexus-WordPress'],'body'=>$body,'timeout'=>20,'redirection'=>0,'limit_response_size'=>65536]);
        if (is_wp_error($response) || wp_remote_retrieve_response_code($response)!==200) { return new WP_Error('oauth_unavailable','GitHub sign-in is unavailable.',['status'=>502]); }
        $data=json_decode(wp_remote_retrieve_body($response),true);
        return is_array($data)?$data:new WP_Error('oauth_response','GitHub returned an invalid sign-in response.',['status'=>502]);
    }
    public static function start() {
        $id=Nexus_Store::setting('github_client_id');
        if (!$id || !Nexus_Store::setting('github_client_secret')) { return new WP_Error('auth_unconfigured','Configure the GitHub OAuth application in WordPress first.',['status'=>503]); }
        // Use the actual peer address, never client-supplied forwarded headers.
        $rate='nexus_login_'.hash('sha256',$_SERVER['REMOTE_ADDR']??'unknown');
        $count=(int)get_transient($rate);
        if ($count>=20) { return new WP_Error('login_limit','Too many sign-in attempts. Try again later.',['status'=>429]); }
        set_transient($rate,$count+1,900);
        $data=self::oauth('login/device/code',['client_id'=>$id]);
        if (is_wp_error($data)) { return $data; }
        if (empty($data['device_code']) || empty($data['user_code'])) { return new WP_Error('device_disabled','Enable Device Flow in the GitHub OAuth application.',['status'=>503]); }
        $poll=bin2hex(random_bytes(32)); $interval=max(1,(int)($data['interval']??5)); $expires=min(900,max(1,(int)($data['expires_in']??900)));
        Nexus_Store::cleanup();
        Nexus_Store::put('flow',hash('sha256',$poll),['device_code'=>$data['device_code'],'interval'=>$interval,'next'=>time()],time()+$expires);
        // Never forward arbitrary OAuth verification URLs.
        return ['poll_id'=>$poll,'user_code'=>$data['user_code'],'verification_uri'=>'https://github.com/login/device','verification_uri_complete'=>'https://github.com/login/device?user_code='.rawurlencode($data['user_code']),'interval_seconds'=>$interval,'expires_in'=>$expires];
    }
    public static function poll($request) {
        $poll=(string)$request->get_param('poll_id');
        if (!preg_match('/^[a-f0-9]{64}$/D',$poll)) { return ['status'=>'expired']; }
        $hash=hash('sha256',$poll); $flow=Nexus_Store::get('flow',$hash);
        if (!$flow) { return ['status'=>'expired']; }
        if ($flow['next']>time()) { return ['status'=>'pending','interval_seconds'=>$flow['interval']]; }
        $claimed=$flow; $claimed['next']=time()+$flow['interval'];
        if (!Nexus_Store::claim_flow($hash,$flow,$claimed,0)) { return ['status'=>'pending','interval_seconds'=>$flow['interval']]; }
        $data=self::oauth('login/oauth/access_token',['client_id'=>Nexus_Store::setting('github_client_id'),'client_secret'=>Nexus_Store::setting('github_client_secret'),'device_code'=>$flow['device_code'],'grant_type'=>'urn:ietf:params:oauth:grant-type:device_code']);
        if (is_wp_error($data)) { return $data; }
        if (isset($data['access_token'])) {
            $user=Nexus_Github::json('/user',false,$data['access_token']);
            unset($data['access_token']); // GitHub user tokens are not persisted or sent to the desktop.
            if (is_wp_error($user) || empty($user['id']) || empty($user['login'])) { return new WP_Error('identity_failed','Could not verify your GitHub identity.',['status'=>502]); }
            Nexus_Store::delete('flow',$hash);
            $session=bin2hex(random_bytes(32)); $identity=['github_id'=>(int)$user['id'],'login'=>$user['login']];
            Nexus_Store::put('session',hash('sha256',$session),$identity,time()+30*DAY_IN_SECONDS);
            return ['status'=>'approved','session_token'=>$session,'account'=>self::account($identity)];
        }
        switch ($data['error']??'') {
            case 'authorization_pending': return ['status'=>'pending','interval_seconds'=>$flow['interval']];
            case 'slow_down':
                $claimed['interval']=min(60,$flow['interval']+5); $claimed['next']=time()+$claimed['interval'];
                // Preserve the original expiration; do not extend a device flow by polling.
                global $wpdb;
                $wpdb->update(Nexus_Store::table(),['body'=>wp_json_encode($claimed)],['record_kind'=>'flow','record_id'=>$hash],['%s'],['%s','%s']);
                return ['status'=>'slow_down','interval_seconds'=>$claimed['interval']];
            case 'access_denied': Nexus_Store::delete('flow',$hash); return ['status'=>'denied'];
            case 'expired_token': Nexus_Store::delete('flow',$hash); return ['status'=>'expired'];
            default: return new WP_Error('oauth_failed','GitHub did not complete sign-in.',['status'=>502]);
        }
    }
    public static function logout($request) {
        $token=substr($request->get_header('authorization'),7);
        Nexus_Store::delete('session',hash('sha256',$token));
        return new WP_REST_Response(null,204);
    }
}
