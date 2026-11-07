<?php
// Creates a disposable WordPress install. Configuration comes from CI/local test environment.
$root=getenv('NEXUS_TEST_WP_ROOT');
if (!$root || getenv('NEXUS_TEST_ALLOW_WRITE')!=='yes') { fwrite(STDERR,"Disposable test root and explicit write opt-in required.\n"); exit(2); }
if (!file_exists($root.'/wp-config.php')) {
    $config="<?php\n";
    foreach (['DB_NAME'=>'nexus_wp_test','DB_USER'=>'root','DB_PASSWORD'=>'nexus-test-pass','DB_HOST'=>'127.0.0.1:3306','DB_CHARSET'=>'utf8mb4','DB_COLLATE'=>'','AUTH_KEY'=>bin2hex(random_bytes(32)),'WP_SITEURL'=>'http://127.0.0.1:8899','WP_HOME'=>'http://127.0.0.1:8899'] as $key=>$default) {
        $value=getenv('NEXUS_TEST_'.$key); $config.='define('.var_export($key,true).','.var_export($value===false?$default:$value,true).");\n";
    }
    $config.="define('DISABLE_WP_CRON',true);\n\$table_prefix='nxt_';\nif(!defined('ABSPATH')) define('ABSPATH',__DIR__.'/');\nrequire_once ABSPATH.'wp-settings.php';\n";
    file_put_contents($root.'/wp-config.php',$config); chmod($root.'/wp-config.php',0600);
}
$_SERVER['HTTP_HOST']='127.0.0.1:8899';
define('WP_INSTALLING',true);
require $root.'/wp-load.php';
require_once ABSPATH.'wp-admin/includes/upgrade.php';
add_filter('pre_wp_mail','__return_true');
if (!is_blog_installed()) { wp_install('Nexus disposable test','nexus-test-admin','test@example.invalid',false,'','temporary-nexus-test-password'); }
require_once ABSPATH.'wp-admin/includes/plugin.php';
$result=activate_plugin('nexus-service/nexus-service.php');
if (is_wp_error($result)) { fwrite(STDERR,$result->get_error_message()); exit(1); }
update_option('permalink_structure','/%postname%/'); flush_rewrite_rules();
echo "Disposable WordPress installed and Nexus Service activated.\n";
