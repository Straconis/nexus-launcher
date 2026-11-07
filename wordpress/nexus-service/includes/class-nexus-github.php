<?php
if (!defined('ABSPATH')) { exit; }
final class Nexus_Github {
    public static function request(string $path, bool $private=false, string $accept='application/vnd.github+json', ?string $user_token=null) {
        // Paths are constructed from validated repo names or numeric asset ids; never arbitrary URLs.
        if (!str_starts_with($path, '/') || str_contains($path, '..') || str_contains($path, '?') || str_contains($path, '#')) {
            return new WP_Error('invalid_path', 'Invalid GitHub path.', ['status'=>400]);
        }
        $headers = ['Accept'=>$accept,'User-Agent'=>'Straconis-Nexus-WordPress','X-GitHub-Api-Version'=>'2022-11-28'];
        $token = $user_token ?? Nexus_Store::setting('github_release_token');
        $id = Nexus_Store::setting('github_client_id'); $secret = Nexus_Store::setting('github_client_secret');
        if ($token !== '') { $headers['Authorization'] = 'Bearer '.$token; }
        elseif ($private) { return new WP_Error('release_token_missing','Private repositories need a server-side release credential.',['status'=>503]); }
        elseif ($id !== '' && $secret !== '') { $headers['Authorization'] = 'Basic '.base64_encode($id.':'.$secret); }
        $response = wp_remote_get('https://api.github.com'.$path, ['headers'=>$headers,'timeout'=>25,'redirection'=>0,'limit_response_size'=>4*1024*1024]);
        if (is_wp_error($response)) { return new WP_Error('github_unavailable','GitHub is temporarily unavailable.',['status'=>502]); }
        return $response;
    }
    public static function json(string $path, bool $private=false, ?string $user_token=null) {
        $response = self::request($path, $private, 'application/vnd.github+json', $user_token);
        if (is_wp_error($response)) { return $response; }
        $code = wp_remote_retrieve_response_code($response);
        if ($code !== 200) { return new WP_Error('github_unavailable', 'GitHub lookup unavailable. Check server credentials and repository access.', ['status'=>$code===404?404:502]); }
        $json = json_decode(wp_remote_retrieve_body($response), true);
        return is_array($json) ? $json : new WP_Error('github_response','GitHub returned an invalid response.',['status'=>502]);
    }
    public static function languages(array $counts): array {
        $names = array_keys(array_filter($counts, fn($count)=>is_numeric($count) && $count>0));
        usort($names, fn($a,$b)=>($counts[$b]<=>$counts[$a]) ?: strcmp($a,$b));
        return array_slice($names,0,3);
    }
    public static function metadata(array $app) {
        $repo = $app['repo']; $key = hash('sha256',$repo); $cached=Nexus_Store::get('metadata',$key,true);
        if ($cached && $cached['checked_at'] > time()-300) { return $cached['data']; }
        // One upstream refresh per repository at a time, even if several launchers refresh together.
        $lock='nexus_refresh_'.$key;
        if (!add_option($lock, time()+60, '', false)) {
            if ((int)get_option($lock,0)<time()) { delete_option($lock); }
            if ($cached) { $cached['data']['stale']=true; return $cached['data']; }
            return new WP_Error('metadata_busy','Release information is refreshing. Try again shortly.',['status'=>503]);
        }
        try {
            $release=self::json('/repos/'.$repo.'/releases/latest');
            if (is_wp_error($release) && $release->get_error_data()['status']!==404) {
                if ($cached) { $cached['data']['stale']=true; return $cached['data']; }
                return $release;
            }
            $languages=self::json('/repos/'.$repo.'/languages');
            $data=['release_json'=>is_wp_error($release)?null:wp_json_encode($release), 'languages'=>is_wp_error($languages)?($cached['data']['languages']??[]):self::languages($languages),'stale'=>false];
            Nexus_Store::put('metadata',$key,['checked_at'=>time(),'data'=>$data]);
            return $data;
        } finally { delete_option($lock); }
    }
}
