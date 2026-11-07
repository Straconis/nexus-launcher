<?php
// Run against a disposable WordPress database, never production.
$load=getenv('NEXUS_TEST_WP_LOAD');
if (!$load || getenv('NEXUS_TEST_ALLOW_WRITE')!=='yes') { fwrite(STDERR,"Set NEXUS_TEST_WP_LOAD and NEXUS_TEST_ALLOW_WRITE=yes for a disposable WordPress installation.\n"); exit(2); }
require $load;
require_once ABSPATH.'wp-admin/includes/plugin.php';
activate_plugin('nexus-service/nexus-service.php');
global $wpdb;
$wpdb->query('DELETE FROM '.Nexus_Store::table());
foreach (['categories','project_types','status_stamps'] as $key) { delete_option('nexus_'.$key); }
Nexus_Store::install();
$assertions=0;
function check($condition,string $message): void { global $assertions; $assertions++; if (!$condition) { throw new RuntimeException($message); } }
function api(string $method,string $path,?array $body=null,?string $token=null): WP_REST_Response {
    [$path,$query]=array_pad(explode('?', $path, 2),2,'');
    $request=new WP_REST_Request($method,'/nexus/v1'.$path);
    if ($query) { parse_str($query,$params); $request->set_query_params($params); }
    if ($body!==null) { $request->set_header('Content-Type','application/json'); $request->set_body(wp_json_encode($body)); }
    if ($token) { $request->set_header('Authorization','Bearer '.$token); }
    return rest_do_request($request);
}
function reply($body,int $code=200,array $headers=[]): array { return ['headers'=>$headers,'body'=>is_string($body)?$body:wp_json_encode($body),'response'=>['code'=>$code,'message'=>'test'],'cookies'=>[]]; }
$github_calls=0; $fail_github=false; $polls=0; $seen_headers=[];
$tamper=false; $bad_redirect=false;
$bytes='verified-test-release'; $digest=hash('sha256',$bytes);
add_filter('pre_http_request',function($pre,$args,$url) use (&$github_calls,&$fail_github,&$polls,&$seen_headers,&$tamper,&$bad_redirect,$bytes,$digest) {
    $seen_headers[$url]=$args['headers'];
    if (str_contains($url,'github.com/login/device/code')) { return reply(['device_code'=>'private-device-code','user_code'=>'TEST-CODE','interval'=>1,'expires_in'=>900]); }
    if (str_contains($url,'github.com/login/oauth/access_token')) {
        $polls++;
        return reply($polls===1?['error'=>'authorization_pending']:($polls===2?['error'=>'slow_down']:['access_token'=>'private-user-token']));
    }
    if ($url==='https://api.github.com/user') { return reply(['id'=>42,'login'=>'admin-test']); }
    if (str_contains($url,'/users/member')) { return reply(['id'=>99,'login'=>'member']); }
    if (str_contains($url,'/releases/latest')) {
        $github_calls++;
        if ($fail_github) { return reply(['message'=>'private-upstream-detail'],403); }
        return reply(['tag_name'=>'v1.0.0','assets'=>[
            ['id'=>1,'name'=>'test-linux-x86_64.tar.gz','size'=>strlen($bytes),'digest'=>'sha256:'.$digest,'browser_download_url'=>'https://github.com/Straconis/test/releases/download/v1/test-linux-x86_64.tar.gz'],
            ['id'=>2,'name'=>'test-windows-x86_64.zip','size'=>strlen($bytes),'digest'=>'sha256:'.$digest,'browser_download_url'=>'https://github.com/Straconis/test/releases/download/v1/test-windows-x86_64.zip']
        ]]);
    }
    if (str_ends_with($url,'/languages')) { return reply(['Shell'=>20,'Python'=>500,'Rust'=>100]); }
    if (str_contains($url,'api.github.com/repos/') && str_contains($url,'/releases/assets/')) { return reply('',302,['location'=>$bad_redirect?'https://example.invalid/private':'https://release-assets.githubusercontent.com/test-release']); }
    if ($url==='https://release-assets.githubusercontent.com/test-release') { file_put_contents($args['filename'],$tamper?'tampered':$bytes); return reply(''); }
    return new WP_Error('unexpected_network','Unexpected test network request.');
},10,3);
update_option('nexus_github_client_id','test-client',false);
update_option('nexus_github_client_secret','private-client-secret',false);
update_option('nexus_github_release_token','private-release-token',false);
update_option('nexus_admin_github_ids','42',false);
$admin=str_repeat('a',64); $member=str_repeat('b',64);
Nexus_Store::put('session',hash('sha256',$admin),['github_id'=>42,'login'=>'admin-test'],time()+3600);
Nexus_Store::put('session',hash('sha256',$member),['github_id'=>99,'login'=>'member'],time()+3600);
$public=['id'=>'test-public','name'=>'Test public','repo'=>'Straconis/test','visibility'=>'public','published'=>true,'kind'=>'app','summary'=>str_repeat('a',407),'category'=>'Hardware / Electronics'];
check(api('GET','/health')->get_data()['status']==='ok','health');
check(api('GET','/admin/apps')->get_status()===401,'anonymous admin rejected');
check(api('POST','/admin/apps',$public,$member)->get_status()===403,'member admin rejected');
check(api('POST','/admin/apps',$public,$admin)->get_status()===200,'407-character summary saved');
$unicode=$public; $unicode['summary']=str_repeat('🔧',1001);
check(api('POST','/admin/apps',$unicode,$admin)->get_status()===200,'Unicode character count');
$long=$public; $long['summary']=str_repeat('a',4001);
check(api('POST','/admin/apps',$long,$admin)->get_status()===400,'long summary rejected');
check(api('GET','/admin/apps',null,$admin)->get_data()[0]['summary']===$unicode['summary'],'failed write preserves saved summary');
$private=$public; $private['id']='test-private'; $private['repo']='Straconis/private'; $private['visibility']='private';
check(api('POST','/admin/apps',$private,$admin)->get_status()===200,'private app saved');
check(count(api('GET','/catalog')->get_data())===1,'anonymous catalog hides private');
check(api('GET','/apps/test-private/public-metadata')->get_status()===404,'private metadata anonymous rejected');
check(api('GET','/apps/test-private/download?os=linux&arch=x86_64')->get_status()!==200,'private download rejected');
check(api('POST','/admin/apps/test-private/grants',['login'=>'member'],$admin)->get_status()===200,'grant creation');
$catalog=api('GET','/catalog',null,$member)->get_data();
check(count($catalog)===2,'grant unlocks catalog');
foreach ($catalog as $app) { if ($app['visibility']==='private') { check(!isset($app['repo']),'private repo omitted'); } }
check(api('GET','/apps/test-private/public-metadata',null,$member)->get_status()===403,'private raw metadata rejected');
$private_meta=api('GET','/apps/test-private/release?os=linux&arch=x86_64',null,$member);
check($private_meta->get_status()===200 && !isset($private_meta->get_data()['repo']) && !isset($private_meta->get_data()['url']),'private release metadata is limited to install fields');
check(api('DELETE','/admin/apps/test-private/grants/99',null,$admin)->get_status()===204,'revoke grant');
check(count(api('GET','/catalog',null,$member)->get_data())===1,'revocation immediately applied');
$github_calls=0;
$meta=api('GET','/apps/test-public/public-metadata');
check($meta->get_status()===200,'public metadata');
check($meta->get_data()['languages']===['Python','Rust','Shell'],'language ranking');
api('GET','/apps/test-public/public-metadata'); check($github_calls===1,'metadata cached');
$key=hash('sha256','Straconis/test'); $cached=Nexus_Store::get('metadata',$key,true); $cached['checked_at']=time()-301; Nexus_Store::put('metadata',$key,$cached);
$fail_github=true;
check(api('GET','/apps/test-public/public-metadata')->get_data()['stale']===true,'stale cache fallback');
$fail_github=false;
check(api('POST','/admin/apps/test-public/published',['published'=>false],$admin)->get_status()===204,'hide listing');
check(api('GET','/apps/test-public/public-metadata')->get_status()===404,'hidden app blocked before cache');
api('POST','/admin/apps/test-public/published',['published'=>true],$admin);
check(api('POST','/admin/categories',['name'=>'','replacement'=>'Hardware / New'],$admin)->get_status()===200,'add category');
check(api('POST','/admin/categories',['name'=>'Hardware','replacement'=>'Devices'],$admin)->get_status()===200,'rename category branch');
check(Nexus_Store::get('app','test-public')['category']==='Devices / Electronics','branch assignments renamed');
check(api('POST','/admin/categories',['name'=>'Devices','replacement'=>'Devices / Cycle'],$admin)->get_status()===400,'category cycles blocked');
$supported=['id'=>'test-supported','name'=>'Supported','kind'=>'supported','homepage'=>'https://example.com','visibility'=>'public','category'=>'Podcast','published'=>true];
check(api('POST','/admin/apps',$supported,$admin)->get_status()===200,'supported project');
api('POST','/admin/project-types',['name'=>'Podcast','replacement'=>'Shows'],$admin);
check(Nexus_Store::get('app','test-supported')['category']==='Shows','type rename');
check(Nexus_Store::get('app','test-public')['category']==='Devices / Electronics','tool categories unaffected');
check(api('POST','/admin/status-stamps',['id'=>'prototype','name'=>'Seeking funding for prototype','color'=>'#12abef'],$admin)->get_status()===200,'custom stamp');
check(api('POST','/admin/apps/test-public/status',['project_status'=>'prototype'],$admin)->get_status()===200,'stamp assigned');
check(api('POST','/admin/apps/test-public/status',['project_status'=>'prototype'],$admin)->get_data()===['saved'=>true],'status response matches desktop JSON contract');
$listed=api('GET','/catalog')->get_data();
check(count(array_filter($listed,fn($a)=>isset($a['status_stamp']['color']) && $a['status_stamp']['color']==='#12abef'))===1,'colored stamp in catalog');
$before=Nexus_Store::get('app','test-public');
$invalid=Nexus_Admin::import(['apps'=>[$public,$long]]);
check(is_wp_error($invalid),'invalid import rejected');
check(Nexus_Store::get('app','test-public')===$before,'invalid import does not partially update');
$start=api('POST','/auth/device/start',[])->get_data();
check(!isset($start['device_code']),'device code remains server-side');
$flow_hash=hash('sha256',$start['poll_id']);
foreach (['pending','slow_down','approved'] as $expected) {
    $flow=Nexus_Store::get('flow',$flow_hash); $flow['next']=0; Nexus_Store::put('flow',$flow_hash,$flow,time()+900);
    $result=api('POST','/auth/device/poll',['poll_id'=>$start['poll_id']])->get_data();
    check($result['status']===$expected,'device poll '.$expected);
}
check($result['account']['admin']===true,'numeric GitHub admin identity');
check(api('GET','/account',null,$result['session_token'])->get_status()===200,'session survives follow-up request');
check(api('POST','/auth/device/poll',['poll_id'=>$start['poll_id']])->get_data()['status']==='expired','device approval cannot be replayed');
check(api('POST','/auth/logout',[],$result['session_token'])->get_status()===204,'logout');
check(api('GET','/account',null,$result['session_token'])->get_status()===401,'logout revokes session');
$request=new WP_REST_Request('GET','/nexus/v1/apps/test-public/release'); $request->set_query_params(['os'=>'linux','arch'=>'x86_64']);
$release=rest_do_request($request);
check($release->get_status()===200 && $release->get_data()['sha256']===$digest,'release metadata checksum');
$request=new WP_REST_Request('GET','/nexus/v1/apps/test-public/download'); $request->set_query_params(['os'=>'linux','arch'=>'x86_64']);
$download=rest_do_request($request);
check($download instanceof Nexus_Download_Response,'binary download response');
check(file_get_contents($download->file)===$bytes,'download bytes verified'); unlink($download->file);
check(!isset($seen_headers['https://release-assets.githubusercontent.com/test-release']['Authorization']),'credentials stripped on asset redirect');
$tamper=true;
check(rest_do_request($request)->get_status()===502,'tampered download rejected');
$tamper=false; $bad_redirect=true;
check(rest_do_request($request)->get_status()===502,'untrusted redirect rejected');
$bad_redirect=false;
check(Nexus_Releases::sums($digest.' *test.zip','test.zip')===$digest,'checksum sum format');
check(Nexus_Releases::select([['name'=>'test-linux-aarch64.tar.gz']],'linux','x86_64')===null,'architecture mismatch rejected');
$expired=str_repeat('e',64); Nexus_Store::put('session',hash('sha256',$expired),['github_id'=>42,'login'=>'admin-test'],time()-1);
check(api('GET','/admin/apps',null,$expired)->get_status()===401,'expired admin session rejected');
$serialized=wp_json_encode(api('GET','/catalog')->get_data());
foreach (['private-client-secret','private-device-code','private-user-token','private-release-token','Straconis/private'] as $secret) { check(!str_contains($serialized,$secret),'catalog excludes secret '.$secret); }
echo "PASS: $assertions WordPress integration assertions.\n";
