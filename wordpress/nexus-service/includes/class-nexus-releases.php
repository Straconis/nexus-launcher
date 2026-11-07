<?php
if (!defined('ABSPATH')) { exit; }
final class Nexus_Download_Response extends WP_REST_Response {
    public string $file;
    public function __construct(string $file,array $meta) {
        parent::__construct(null,200,['Content-Type'=>'application/octet-stream','Content-Length'=>(string)filesize($file),'x-nexus-asset-name'=>$meta['asset_name'],'x-nexus-sha256'=>$meta['sha256'],'x-nexus-tag'=>$meta['tag'],'Cache-Control'=>'private, no-store']);
        $this->file=$file;
    }
}
final class Nexus_Releases {
    public static function select(array $assets,string $os,string $arch): ?array {
        $arch=match(strtolower($arch)) {'amd64','x64','x86_64'=>'x86_64','arm64','aarch64'=>'aarch64',default=>''};
        $best=null; $score=-1;
        foreach ($assets as $asset) {
            $name=strtolower($asset['name']);
            $os_match=match($os) {
                'linux'=>str_contains($name,'linux') || preg_match('/\.(deb|appimage)$/D',$name),
                'windows'=>str_contains($name,'windows') || str_contains($name,'win64') || preg_match('/\.(exe|msi)$/D',$name),
                'macos'=>str_contains($name,'macos') || str_contains($name,'darwin') || str_ends_with($name,'.dmg'),default=>false};
            $arch_match=($arch==='x86_64' && preg_match('/x86_64|amd64|x64/',$name)) || ($arch==='aarch64' && preg_match('/aarch64|arm64/',$name));
            if (!$os_match || !$arch_match || !preg_match('/\.(zip|tar\.gz|tgz|exe|msi|appimage|deb|dmg)$/D',$name)) { continue; }
            $value=(($os==='linux' && preg_match('/\.(tar\.gz|tgz)$/D',$name)) || ($os==='windows' && str_ends_with($name,'.zip')))?50:(preg_match('/\.(zip|tar\.gz|tgz)$/D',$name)?40:(preg_match('/\.(msi|deb|dmg)$/D',$name)?10:20));
            if ($value>$score) { $score=$value; $best=$asset; }
        }
        return $best;
    }
    public static function sums(string $text,string $filename): ?string {
        foreach (preg_split('/\R/',$text) as $line) {
            if (preg_match('/^([a-fA-F0-9]{64})\s+\*?(.+)$/D',trim($line),$match) && strcasecmp(trim($match[2]),$filename)===0) { return strtolower($match[1]); }
            if (preg_match('/^SHA256\s*\((.+)\)\s*=\s*([a-fA-F0-9]{64})$/Di',trim($line),$match) && strcasecmp($match[1],$filename)===0) { return strtolower($match[2]); }
        }
        return null;
    }
    public static function asset_file(array $app,array $asset,int $limit) {
        if (empty($asset['id']) || !ctype_digit((string)$asset['id']) || (int)($asset['size']??0)>$limit) { return Nexus_API::error('Release asset is invalid or exceeds the server download limit.',502); }
        require_once ABSPATH.'wp-admin/includes/file.php';
        $file=wp_tempnam('nexus-asset'); if (!$file) { return Nexus_API::error('Could not create a temporary download.',503); }
        $url='https://api.github.com/repos/'.$app['repo'].'/releases/assets/'.$asset['id'];
        $headers=['User-Agent'=>'Straconis-Nexus-WordPress','Accept'=>'application/octet-stream'];
        $token=Nexus_Store::setting('github_release_token');
        if ($token!=='') { $headers['Authorization']='Bearer '.$token; }
        elseif ($app['visibility']==='private') { unlink($file); return Nexus_API::error('Private repositories require a server release credential.',503); }
        for ($hop=0;$hop<4;$hop++) {
            $response=wp_remote_get($url,['headers'=>$headers,'timeout'=>90,'redirection'=>0,'stream'=>true,'filename'=>$file,'limit_response_size'=>$limit+1]);
            if (is_wp_error($response)) { unlink($file); return Nexus_API::error('Release download failed.',502); }
            $code=wp_remote_retrieve_response_code($response);
            if ($code===200) {
                clearstatcache(true,$file);
                if (filesize($file)>$limit) { unlink($file); return Nexus_API::error('Release file exceeds the download limit.',502); }
                return $file;
            }
            if (!in_array($code,[301,302,303,307,308],true)) { break; }
            $url=wp_remote_retrieve_header($response,'location'); $parsed=wp_parse_url($url);
            if (!$parsed || ($parsed['scheme']??'')!=='https' || isset($parsed['user']) || isset($parsed['pass']) || (isset($parsed['port']) && $parsed['port']!==443) || !in_array($parsed['host']??'',['release-assets.githubusercontent.com','objects.githubusercontent.com','github.com'],true)) { break; }
            // Signed asset redirects never receive the GitHub credential.
            unset($headers['Authorization']);
        }
        unlink($file); return Nexus_API::error('GitHub did not provide a permitted release download.',502);
    }
    private static function resolve($request) {
        $app=Nexus_API::load($request); if (is_wp_error($app)) { return $app; }
        if ($app['kind']!=='app' || !$app['repo']) { return Nexus_API::error('This project has no software release.',404); }
        $os=strtolower((string)$request->get_param('os')); $arch=(string)$request->get_param('arch');
        if ($app['visibility']==='public') {
            $meta=Nexus_Github::metadata($app); if (is_wp_error($meta)) { return $meta; }
            $release=json_decode($meta['release_json']??'null',true);
        } else {
            $key=hash('sha256',$app['repo']); $cached=Nexus_Store::get('private_release',$key);
            $release=$cached??Nexus_Github::json('/repos/'.$app['repo'].'/releases/latest',true);
            if (!is_wp_error($release)) { Nexus_Store::put('private_release',$key,$release,time()+300); }
        }
        if (is_wp_error($release)) { return $release; }
        if (!$release) { return Nexus_API::error('No release published.',404); }
        $asset=self::select($release['assets']??[],$os,$arch);
        if (!$asset) { return Nexus_API::error('No release asset for this platform.',404); }
        if (!preg_match('/^[A-Za-z0-9_.-]+$/D',$asset['name']) || !preg_match('/^[\x20-\x7e]+$/D',$release['tag_name'])) { return Nexus_API::error('Release headers contain unsupported characters.',502); }
        $digest=preg_replace('/^sha256:/i','',(string)($asset['digest']??''));
        if (!preg_match('/^[a-fA-F0-9]{64}$/D',$digest)) {
            $digest=null;
            foreach ($release['assets']??[] as $candidate) {
                if (!in_array(strtolower($candidate['name']),['sha256sums','sha256sums.txt','sha256sum.txt','checksums.txt'],true)) { continue; }
                $file=self::asset_file($app,$candidate,1024*1024); if (is_wp_error($file)) { return $file; }
                try { $digest=self::sums(file_get_contents($file),$asset['name']); } finally { unlink($file); }
                break;
            }
        }
        if (!$digest) { return Nexus_API::error('The release must publish a SHA-256 checksum.',409); }
        $name=strtolower($asset['name']);
        $kind=preg_match('/\.(zip|tar\.gz|tgz)$/D',$name)?'archive':(preg_match('/\.(exe|msi|deb|dmg)$/D',$name)?'handoff':'portable');
        return [$app,$asset,['tag'=>$release['tag_name'],'asset_name'=>$asset['name'],'size'=>(int)$asset['size'],'sha256'=>strtolower($digest),'kind'=>$kind]];
    }
    public static function release($request) { $result=self::resolve($request); return is_wp_error($result)?$result:$result[2]; }
    public static function download($request) {
        $result=self::resolve($request); if (is_wp_error($result)) { return $result; }
        [$app,$asset,$meta]=$result;
        $file=self::asset_file($app,$asset,256*1024*1024); if (is_wp_error($file)) { return $file; }
        if (!hash_equals($meta['sha256'],hash_file('sha256',$file))) { unlink($file); return Nexus_API::error('Release checksum mismatch.',502); }
        return new Nexus_Download_Response($file,$meta);
    }
}
