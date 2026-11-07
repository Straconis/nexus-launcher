<?php
if (!defined('ABSPATH')) { exit; }
final class Nexus_API {
    public static function rewrites(): void {
        // Existing 0.3.0 launchers use /v1 at the service hostname root.
        add_rewrite_rule('^v1/(.*)$','index.php?rest_route=/nexus/v1/$matches[1]','top');
    }
    public static function error(string $message,int $status=400) { return new WP_Error('nexus_error',$message,['status'=>$status]); }
    public static function routes(): void {
        $routes=[
            ['/health','GET','health',false], ['/catalog','GET','catalog',false],
            ['/auth/device/start','POST','start',false], ['/auth/device/poll','POST','poll',false],
            ['/auth/logout','POST','logout',true], ['/account','GET','account',true],
            ['/apps/(?P<id>[a-z0-9-]+)/public-metadata','GET','metadata',false],
            ['/apps/(?P<id>[a-z0-9-]+)/languages','GET','languages',false],
            ['/apps/(?P<id>[a-z0-9-]+)/release','GET','release',false],
            ['/apps/(?P<id>[a-z0-9-]+)/download','GET','download',false],
            ['/admin/apps','GET,POST','apps',true], ['/admin/apps/(?P<id>[a-z0-9-]+)','DELETE','delete_app',true],
            ['/admin/apps/(?P<id>[a-z0-9-]+)/published','POST','publish',true],
            ['/admin/apps/(?P<id>[a-z0-9-]+)/status','POST','status',true],
            ['/admin/apps/(?P<id>[a-z0-9-]+)/grants','GET,POST','grants',true],
            ['/admin/apps/(?P<id>[a-z0-9-]+)/grants/(?P<github_id>[0-9]+)','DELETE','revoke',true],
            ['/admin/categories','GET,POST','categories',true], ['/admin/project-types','GET,POST','project_types',true],
            ['/admin/status-stamps','GET,POST','stamps',true], ['/admin/repo-preview','POST','preview',true],
        ];
        foreach ($routes as [$path,$methods,$method,$auth]) {
            $class=in_array($method,['start','poll','logout'],true)?Nexus_Auth::class:self::class;
            register_rest_route('nexus/v1',$path,['methods'=>$methods,'callback'=>function($request) use ($class,$method) {
                try { return $class::$method($request); }
                catch (Throwable $e) { return new WP_Error('nexus_internal','The Nexus service could not complete the request.',['status'=>500]); }
            },'permission_callback'=>$auth?[Nexus_Auth::class,'permission']:'__return_true']);
        }
    }
    public static function health(): array { return ['status'=>'ok','version'=>'0.4.0-alpha','service'=>'wordpress','plugin_version'=>'0.2.0']; }
    public static function account($request) { return Nexus_Auth::caller($request); }
    public static function visible(array $app,?array $caller): bool {
        return $app['published'] && ($app['visibility']==='public' || ($caller && ($caller['admin'] || Nexus_Store::get('grant',$app['id'].':'.$caller['github_id']))));
    }
    public static function load($request) {
        $caller=Nexus_Auth::caller($request); if (is_wp_error($caller)) { return $caller; }
        $app=Nexus_Store::get('app',(string)$request['id']);
        return $app && self::visible($app,$caller)?$app:self::error('Project not found.',404);
    }
    public static function catalog($request) {
        $caller=Nexus_Auth::caller($request); if (is_wp_error($caller)) { return $caller; }
        $stamps=array_column(get_option('nexus_status_stamps',[]),null,'id'); $out=[];
        foreach (Nexus_Store::all('app') as $app) {
            if (!self::visible($app,$caller)) { continue; }
            unset($app['published']);
            if ($app['visibility']==='private') { unset($app['repo']); }
            $app['status_stamp']=$stamps[$app['project_status']??'']??null;
            $out[]=$app;
        }
        return $out;
    }
    public static function metadata($request) {
        $app=self::load($request); if (is_wp_error($app)) { return $app; }
        if ($app['visibility']!=='public' || $app['kind']!=='app') { return self::error('Public app required.',403); }
        if (!$app['repo']) { return self::error('This app has no repository.',404); }
        return Nexus_Github::metadata($app);
    }
    public static function languages($request) {
        $app=self::load($request); if (is_wp_error($app)) { return $app; }
        if (!$app['repo']) { return []; }
        if ($app['visibility']==='public') { $meta=Nexus_Github::metadata($app); return is_wp_error($meta)?$meta:$meta['languages']; }
        $counts=Nexus_Github::json('/repos/'.$app['repo'].'/languages',true);
        return is_wp_error($counts)?$counts:Nexus_Github::languages($counts);
    }
    public static function validate(array $body, ?array $stamp_ids=null) {
        foreach (['id','name','kind','summary','repo','homepage','banner_url','category','visibility','project_status'] as $field) {
            if (isset($body[$field]) && !is_string($body[$field])) { return self::error('Project text fields must be strings.'); }
        }
        $id=$body['id']??''; $name=trim((string)($body['name']??'')); $kind=$body['kind']??'app';
        if (!is_string($id) || !preg_match('/^[a-z0-9]+(?:-[a-z0-9]+)*$/D',$id) || strlen($id)>64) { return self::error('App id must be a lowercase slug of up to 64 characters.'); }
        if (!$name || mb_strlen($name)>100 || !in_array($kind,['app','supported'],true)) { return self::error('Provide a name of up to 100 characters and a valid project kind.'); }
        $summary=trim((string)($body['summary']??''));
        if (mb_strlen($summary)>4000) { return self::error('Summary must be 4,000 characters or fewer.'); }
        $repo=trim((string)($body['repo']??''));
        if ($repo!=='' && (!preg_match('~^[A-Za-z0-9_-][A-Za-z0-9_.-]{0,99}/[A-Za-z0-9_-][A-Za-z0-9_.-]{0,99}$~D',$repo) || str_contains($repo,'..') || str_ends_with($repo,'.'))) { return self::error('Repository must be owner/name.'); }
        if (($body['visibility']??'private')==='private' && $repo==='') { return self::error('Private apps require a GitHub repository.'); }
        $homepage=trim((string)($body['homepage']??'')); $url=$homepage?wp_parse_url($homepage):[];
        if ($homepage && (!$url || ($url['scheme']??'')!=='https' || empty($url['host']) || isset($url['user']) || isset($url['pass']))) { return self::error('Homepage must be HTTPS without embedded credentials.'); }
        if ($kind==='supported' && (!$homepage || ($body['visibility']??'private')!=='public')) { return self::error('Supported projects need a homepage.'); }
        $visibility=$body['visibility']??'private';
        if (!in_array($visibility,['public','private'],true) || (isset($body['published']) && !is_bool($body['published']))) { return self::error('Invalid visibility or listing value.'); }
        $status=$body['project_status']??null; $stamps=$stamp_ids??array_column(get_option('nexus_status_stamps',[]),'id');
        if ($status!==null && !in_array($status,$stamps,true)) { return self::error('Choose a configured status stamp.'); }
        $category=trim((string)($body['category']??''));
        if (mb_strlen($category)>60 || preg_match('/[\x00-\x1f\x7f]/',$category)) { return self::error('Category must be 60 characters or fewer without control characters.'); }
        $banner=trim((string)($body['banner_url']??'')); $banner_parts=$banner?wp_parse_url($banner):[];
        if ($banner!=='' && (strlen($banner)>2048 || !$banner_parts || strtolower($banner_parts['scheme']??'')!=='https' || empty($banner_parts['host']) || isset($banner_parts['user']) || isset($banner_parts['pass']))) {
            return self::error('Tile banner must be an HTTPS URL without credentials (2,048 bytes or fewer).');
        }
        return ['id'=>$id,'name'=>$name,'kind'=>$kind,'summary'=>$summary,'repo'=>$repo?:null,'homepage'=>$homepage?:null,'banner_url'=>$banner!==''?$banner:null,'category'=>$category?:null,'project_status'=>$status,'visibility'=>$visibility,'published'=>$body['published']??false];
    }
    public static function apps($request) {
        if ($request->get_method()==='GET') { return Nexus_Store::all('app'); }
        $body=$request->get_json_params();
        if (!is_array($body)) { return self::error('A JSON app object is required.'); }
        $app=self::validate($body); if (is_wp_error($app)) { return $app; }
        $existing=Nexus_Store::get('app',$app['id']);
        if ($existing && $existing['kind']!==$app['kind']) { return self::error('That id belongs to a different project kind.',409); }
        if (!isset($body['banner_url'])) { $app['banner_url']=$existing['banner_url']??null; }
        Nexus_Store::put('app',$app['id'],$app);
        if ($app['category']) {
            $key=$app['kind']==='supported'?'nexus_project_types':'nexus_categories'; $list=get_option($key,[]);
            if (!in_array($app['category'],$list,true)) { $list[]=$app['category']; sort($list); update_option($key,$list,false); }
        }
        return $app;
    }
    public static function delete_app($request) {
        $id=(string)$request['id']; Nexus_Store::delete('app',$id);
        foreach (Nexus_Store::all('grant') as $grant) { if ($grant['app_id']===$id) { Nexus_Store::delete('grant',$id.':'.$grant['github_id']); } }
        return new WP_REST_Response(null,204);
    }
    public static function publish($request) {
        $app=Nexus_Store::get('app',(string)$request['id']); if (!$app) { return self::error('Project not found.',404); }
        $value=$request->get_param('published'); if (!is_bool($value)) { return self::error('published must be true or false.'); }
        $app['published']=$value; Nexus_Store::put('app',$app['id'],$app); return new WP_REST_Response(null,204);
    }
    public static function status($request) {
        $app=Nexus_Store::get('app',(string)$request['id']); if (!$app) { return self::error('Project not found.',404); }
        $value=$request->get_param('project_status');
        if ($value!==null && !in_array($value,array_column(get_option('nexus_status_stamps',[]),'id'),true)) { return self::error('Choose a configured status stamp.'); }
        $app['project_status']=$value; Nexus_Store::put('app',$app['id'],$app); return ['saved'=>true];
    }
    public static function grants($request) {
        $id=(string)$request['id']; if (!Nexus_Store::get('app',$id)) { return self::error('Project not found.',404); }
        if ($request->get_method()==='POST') {
            $login=(string)$request->get_param('login'); if (!preg_match('/^[A-Za-z0-9](?:[A-Za-z0-9-]{0,37}[A-Za-z0-9])?$/D',$login)) { return self::error('Invalid GitHub username.'); }
            $user=Nexus_Github::json('/users/'.$login); if (is_wp_error($user)) { return $user; }
            $grant=['github_id'=>(int)$user['id'],'login'=>$user['login']];
            Nexus_Store::put('grant',$id.':'.$grant['github_id'],$grant+['app_id'=>$id]); return $grant;
        }
        return array_values(array_map(fn($g)=>['github_id'=>$g['github_id'],'login'=>$g['login']],array_filter(Nexus_Store::all('grant'),fn($g)=>$g['app_id']===$id)));
    }
    public static function revoke($request) { Nexus_Store::delete('grant',(string)$request['id'].':'.(int)$request['github_id']); return new WP_REST_Response(null,204); }
    public static function categories($request) { return self::change_list($request,false); }
    public static function project_types($request) { return self::change_list($request,true); }
    private static function change_list($request,bool $supported) {
        $key=$supported?'nexus_project_types':'nexus_categories'; $list=get_option($key,[]);
        if ($request->get_method()==='GET') { return $list; }
        $name=trim((string)$request->get_param('name')); $replacement=$request->get_param('replacement');
        if ($replacement!==null) { $replacement=trim((string)$replacement); }
        foreach ([$name,$replacement] as $value) {
            if ($value!==null && $value!=='' && (mb_strlen($value)>60 || preg_match('/[\x00-\x1f\x7f]/',$value) || preg_match('~(^| / )\s*( / |$)~',$value))) { return self::error('Names must be 1–60 characters with valid subcategories.'); }
        }
        if (($name==='' && !$replacement) || $replacement==='') { return self::error('A name is required.'); }
        if ($replacement!==null && $name!=='' && str_starts_with($replacement,$name.' / ')) { return self::error('Cannot move a category into its own subcategory.'); }
        $match=fn($v)=>$v===$name || (!$supported && $name!=='' && str_starts_with((string)$v,$name.' / '));
        $transform=fn($v)=>$replacement===null?null:$replacement.substr($v,strlen($name));
        $next=[]; foreach ($list as $item) { $value=$match($item)?$transform($item):$item; if ($value!==null) { $next[]=$value; } }
        if ($replacement!==null && !in_array($replacement,$next,true)) { $next[]=$replacement; }
        $next=array_values(array_unique($next)); sort($next);
        foreach (Nexus_Store::all('app') as $app) {
            if ($name!=='' && ($app['kind']==='supported')===$supported && $match($app['category']??'')) { $app['category']=$transform($app['category']??''); Nexus_Store::put('app',$app['id'],$app); }
        }
        update_option($key,$next,false); return $next;
    }
    public static function stamps($request) {
        $list=get_option('nexus_status_stamps',[]); if ($request->get_method()==='GET') { return $list; }
        $id=(string)$request->get_param('id'); $name=$request->get_param('name'); $color=$request->get_param('color');
        if (!preg_match('/^[a-z0-9_\-]{1,64}$/D',$id)) { return self::error('Invalid stamp id.'); }
        $list=array_values(array_filter($list,fn($s)=>$s['id']!==$id));
        if ($name!==null) {
            if (!is_string($name) || trim($name)==='' || mb_strlen($name)>60 || !is_string($color) || !preg_match('/^#[a-fA-F0-9]{6}$/D',$color)) { return self::error('Provide a stamp name and #RRGGBB color.'); }
            $list[]=['id'=>$id,'name'=>trim($name),'color'=>$color];
        } else {
            foreach (Nexus_Store::all('app') as $app) { if ($app['project_status']===$id) { $app['project_status']=null; Nexus_Store::put('app',$app['id'],$app); } }
        }
        update_option('nexus_status_stamps',$list,false); return $list;
    }
    public static function preview($request) {
        $repo=(string)$request->get_param('repo');
        $valid=self::validate(['id'=>'preview','name'=>'Preview','repo'=>$repo,'visibility'=>'private']); if (is_wp_error($valid)) { return $valid; }
        $meta=Nexus_Github::json('/repos/'.$repo); if (is_wp_error($meta)) { return $meta; }
        $response=Nexus_Github::request('/repos/'.$repo.'/readme',false,'application/vnd.github.raw+json'); $excerpt='';
        if (!is_wp_error($response) && wp_remote_retrieve_response_code($response)===200) {
            $text=substr(wp_remote_retrieve_body($response),0,65536);
            $text=preg_replace('/```.*?```|~~~.*?~~~/s','',$text);
            foreach (preg_split('/\R\s*\R/',$text) as $paragraph) {
                if (preg_match('/^[#<!\[|*\-]/',trim($paragraph))) { continue; }
                $paragraph=trim(preg_replace('/\s+/u',' ',str_replace(['`','*'],'',$paragraph)));
                if (mb_strlen($paragraph)>=45) { $excerpt=mb_substr($paragraph,0,500); break; }
            }
        }
        $description=trim((string)($meta['description']??''));
        return ['repo'=>$repo,'description'=>$description,'readme_excerpt'=>$excerpt,'summary'=>mb_strlen($description)<45 && $excerpt?$excerpt:$description];
    }
    public static function release($request) { return Nexus_Releases::release($request); }
    public static function download($request) { return Nexus_Releases::download($request); }
}
