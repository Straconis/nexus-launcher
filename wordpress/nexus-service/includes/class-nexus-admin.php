<?php
if (!defined('ABSPATH')) { exit; }
final class Nexus_Admin {
    public static function menu(): void { add_options_page('Nexus Service','Nexus Service','manage_options','nexus-service',[self::class,'page']); }
    private static function names($values): bool {
        if (!is_array($values) || count($values)>500) { return false; }
        foreach ($values as $name) { if (!is_string($name) || trim($name)==='' || mb_strlen($name)>60 || preg_match('/[\x00-\x1f\x7f]/',$name)) { return false; } }
        return true;
    }
    public static function import(array $data) {
        if (!isset($data['apps']) || !is_array($data['apps']) || count($data['apps'])>1000) { return Nexus_API::error('Import needs an apps array of up to 1,000 entries.'); }
        foreach (['categories','project_types'] as $key) { if (isset($data[$key]) && !self::names($data[$key])) { return Nexus_API::error('Invalid category or project type list.'); } }
        $stamps=$data['status_stamps']??get_option('nexus_status_stamps',[]);
        if (!is_array($stamps) || count($stamps)>100) { return Nexus_API::error('Invalid status stamp list.'); }
        $ids=[];
        foreach ($stamps as $stamp) {
            if (!is_array($stamp) || !isset($stamp['id'],$stamp['name'],$stamp['color']) || !is_string($stamp['id']) || !preg_match('/^[a-z0-9_\-]{1,64}$/D',$stamp['id']) || !is_string($stamp['name']) || trim($stamp['name'])==='' || mb_strlen($stamp['name'])>60 || !is_string($stamp['color']) || !preg_match('/^#[a-fA-F0-9]{6}$/D',$stamp['color']) || in_array($stamp['id'],$ids,true)) { return Nexus_API::error('Invalid or duplicate status stamp.'); }
            $ids[]=$stamp['id'];
        }
        $apps=[]; $seen=[];
        foreach ($data['apps'] as $body) {
            if (!is_array($body)) { return Nexus_API::error('Each app must be an object.'); }
            $app=Nexus_API::validate($body,$ids); if (is_wp_error($app)) { return $app; }
            if (isset($seen[$app['id']])) { return Nexus_API::error('Duplicate app id in import.'); }
            $seen[$app['id']]=true; $apps[]=$app;
        }
        // Validate the whole import before writing. Imports upsert; unrelated catalog entries remain intact.
        foreach (['categories','project_types','status_stamps'] as $key) { if (isset($data[$key])) { update_option('nexus_'.$key,$data[$key],false); } }
        foreach ($apps as $app) { Nexus_Store::put('app',$app['id'],$app); }
        return count($apps);
    }
    private static function save_settings(): string {
        $id=trim((string)wp_unslash($_POST['github_client_id']??Nexus_Store::setting('github_client_id')));
        $admins=trim((string)wp_unslash($_POST['admin_github_ids']??Nexus_Store::setting('admin_github_ids')));
        if (!preg_match('/^[a-zA-Z0-9_.-]{1,100}$/D',$id)) { return 'Enter the GitHub OAuth client id.'; }
        if (!preg_match('/^\d+(?:\s*,\s*\d+)*$/D',$admins)) { return 'Enter comma-separated numeric GitHub admin ids.'; }
        update_option('nexus_github_client_id',$id,false); update_option('nexus_admin_github_ids',$admins,false);
        foreach (['github_client_secret','github_release_token'] as $key) {
            $value=trim((string)wp_unslash($_POST[$key]??''));
            if ($value!=='') { update_option('nexus_'.$key,$value,false); }
            if (!empty($_POST['clear_'.$key])) { delete_option('nexus_'.$key); }
        }
        return 'Settings saved. Existing credentials are kept when password fields are blank.';
    }
    public static function page(): void {
        if (!current_user_can('manage_options')) { return; }
        $notice='';
        if (($_SERVER['REQUEST_METHOD']??'')==='POST') {
            check_admin_referer('nexus_service_settings');
            try {
                $action=sanitize_key($_POST['nexus_action']??'');
                if ($action==='settings') { $notice=self::save_settings(); }
                elseif ($action==='import') {
                    $raw=(string)wp_unslash($_POST['catalog_json']??'');
                    if (strlen($raw)>1024*1024) { $notice='Import exceeds 1 MiB.'; }
                    else {
                        $data=json_decode($raw,true); $result=is_array($data)?self::import($data):Nexus_API::error('Invalid JSON.');
                        $notice=is_wp_error($result)?$result->get_error_message():"Imported $result catalog entries.";
                    }
                } elseif ($action==='app') {
                    $body=[];
                    foreach (['id','name','kind','summary','repo','homepage','category','visibility','project_status'] as $key) { $body[$key]=(string)wp_unslash($_POST[$key]??''); }
                    $body['project_status']=$body['project_status']?:null; $body['published']=!empty($_POST['published']);
                    $app=Nexus_API::validate($body);
                    if (is_wp_error($app)) { $notice=$app->get_error_message(); }
                    else { Nexus_Store::put('app',$app['id'],$app); $notice='Project saved.'; }
                }
            } catch (Throwable $e) { $notice='Could not save Nexus settings. Check the database configuration.'; }
        }
        echo '<div class="wrap"><h1>Straconis Nexus Service</h1>';
        if ($notice) { echo '<div class="notice notice-info"><p>'.esc_html($notice).'</p></div>'; }
        echo '<p>Public service: <code>https://nexus.straconis.dev</code>. Health check: <code>https://nexus.straconis.dev/v1/health</code>.</p>';
        echo '<p>DNS, the Hostinger hostname mapping, and HTTPS must be configured separately. Exclude <code>/v1/*</code> and <code>/wp-json/nexus/v1/*</code> from page/CDN caching. Preserve the Authorization header.</p>';
        echo '<h2>GitHub connection</h2><form method="post">'; wp_nonce_field('nexus_service_settings');
        echo '<input type="hidden" name="nexus_action" value="settings"><table class="form-table">';
        foreach (['github_client_id'=>'OAuth client id','github_client_secret'=>'OAuth client secret','github_release_token'=>'Private-release token (optional)','admin_github_ids'=>'GitHub admin numeric ids'] as $key=>$label) {
            $secret=in_array($key,['github_client_secret','github_release_token'],true); $defined=defined('NEXUS_'.strtoupper($key));
            echo '<tr><th><label for="'.esc_attr($key).'">'.esc_html($label).'</label></th><td>';
            if ($defined) { echo '<p>Configured in wp-config.php.</p>'; }
            else {
                echo '<input class="regular-text" id="'.esc_attr($key).'" name="'.esc_attr($key).'" type="'.($secret?'password':'text').'" autocomplete="off" value="'.($secret?'':esc_attr(Nexus_Store::setting($key))).'">';
                if ($secret) { echo '<p>'.(Nexus_Store::setting($key)!==''?'Credential configured; leave blank to keep.':'Not configured.').'</p><label><input type="checkbox" name="clear_'.esc_attr($key).'"> Remove saved credential</label>'; }
            }
            echo '</td></tr>';
        }
        echo '</table>'; submit_button('Save GitHub settings'); echo '</form>';
        $apps=Nexus_Store::all('app');
        echo '<h2>Catalog</h2><p>These entries are also editable from the launcher after GitHub sign-in. Published public entries are visible without sign-in.</p><table class="widefat striped"><thead><tr><th>Project</th><th>Kind</th><th>Visibility</th><th>Listed</th><th></th></tr></thead><tbody>';
        foreach ($apps as $app) {
            $link=add_query_arg(['page'=>'nexus-service','edit'=>$app['id']],admin_url('options-general.php'));
            echo '<tr><td>'.esc_html($app['name']).'</td><td>'.esc_html($app['kind']).'</td><td>'.esc_html($app['visibility']).'</td><td>'.($app['published']?'Yes':'No').'</td><td><a href="'.esc_url($link).'">Edit</a></td></tr>';
        }
        echo '</tbody></table>';
        $edit=isset($_GET['edit'])?Nexus_Store::get('app',sanitize_text_field(wp_unslash($_GET['edit']))):null;
        $edit=$edit??['id'=>'','name'=>'','kind'=>'app','summary'=>'','repo'=>'','homepage'=>'','category'=>'','project_status'=>null,'visibility'=>'public','published'=>false];
        echo '<h2>'.($edit['id']?'Edit project':'Add project').'</h2><form method="post">'; wp_nonce_field('nexus_service_settings');
        echo '<input type="hidden" name="nexus_action" value="app"><table class="form-table">';
        foreach (['id'=>'App id','name'=>'Display name','kind'=>'Project kind','summary'=>'Summary','repo'=>'GitHub owner/repo','homepage'=>'Homepage','category'=>'Category / project type','project_status'=>'Status stamp','visibility'=>'Visibility'] as $key=>$label) {
            echo '<tr><th><label for="app_'.esc_attr($key).'">'.esc_html($label).'</label></th><td>';
            $choices=null;
            if ($key==='kind') { $choices=['app'=>'Software / hardware','supported'=>'Supported project']; }
            if ($key==='visibility') { $choices=['public'=>'Public','private'=>'Private']; }
            if ($key==='project_status') { $choices=[''=>'No stamp']; foreach (get_option('nexus_status_stamps',[]) as $stamp) { $choices[$stamp['id']]=$stamp['name']; } }
            if ($key==='category') { $choices=[''=>'No category']; foreach (get_option($edit['kind']==='supported'?'nexus_project_types':'nexus_categories',[]) as $value) { $choices[$value]=$value; } if ($edit['category']) { $choices[$edit['category']]=$edit['category']; } }
            if ($choices!==null) {
                echo '<select id="app_'.esc_attr($key).'" name="'.esc_attr($key).'">'; foreach ($choices as $value=>$text) { echo '<option value="'.esc_attr($value).'" '.selected($edit[$key]??'',$value,false).'>'.esc_html($text).'</option>'; } echo '</select>';
            } elseif ($key==='summary') { echo '<textarea class="large-text" rows="5" maxlength="4000" id="app_summary" name="summary">'.esc_textarea($edit[$key]).'</textarea>'; }
            else { echo '<input class="regular-text" id="app_'.esc_attr($key).'" name="'.esc_attr($key).'" value="'.esc_attr($edit[$key]??'').'" '.($key==='id' && $edit['id']?'readonly':'').'>'; }
            echo '</td></tr>';
        }
        echo '<tr><th>Listing</th><td><label><input type="checkbox" name="published" '.checked($edit['published'],true,false).'> Show in launcher</label></td></tr></table>';
        submit_button('Save project'); echo '</form>';
        echo '<h2>Import existing catalog</h2><p>Paste the migration JSON from the existing Nexus service. This imports projects, category lists, project types, and stamps. It does not import credentials or sessions. Existing matching ids are updated; unrelated projects remain.</p><form method="post">'; wp_nonce_field('nexus_service_settings');
        echo '<input type="hidden" name="nexus_action" value="import"><textarea class="large-text code" name="catalog_json" rows="8" placeholder="{&quot;apps&quot;: [...]}" required></textarea>';
        submit_button('Import catalog'); echo '</form></div>';
    }
}
