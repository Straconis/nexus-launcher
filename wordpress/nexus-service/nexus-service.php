<?php
/**
 * Plugin Name: Straconis Nexus Service
 * Description: Central catalog, GitHub sign-in, and cached release information for Straconis Nexus Launcher.
 * Version: 0.1.0
 * Requires at least: 6.5
 * Requires PHP: 8.1
 * Author: Straconis Software
 * Author URI: https://straconis.dev
 * License: GPL-2.0-or-later
 */
if (!defined('ABSPATH')) { exit; }
require_once __DIR__ . '/includes/class-nexus-store.php';
require_once __DIR__ . '/includes/class-nexus-github.php';
require_once __DIR__ . '/includes/class-nexus-auth.php';
require_once __DIR__ . '/includes/class-nexus-api.php';
require_once __DIR__ . '/includes/class-nexus-admin.php';
require_once __DIR__ . '/includes/class-nexus-releases.php';
register_activation_hook(__FILE__, function () {
    Nexus_Store::install();
    Nexus_API::rewrites();
    flush_rewrite_rules();
});
register_deactivation_hook(__FILE__, function () {
    global $wp_rewrite;
    unset($wp_rewrite->extra_rules_top['^v1/(.*)$']);
    flush_rewrite_rules();
});
add_action('init', [Nexus_API::class, 'rewrites']);
add_action('rest_api_init', [Nexus_API::class, 'routes']);
add_action('admin_menu', [Nexus_Admin::class, 'menu']);
add_filter('rest_post_dispatch', function ($response, $server, $request) {
    if (str_starts_with($request->get_route(), '/nexus/v1/')) {
        $response->header('Cache-Control', 'private, no-store, max-age=0');
        $response->header('X-Content-Type-Options', 'nosniff');
        do_action('litespeed_control_set_nocache', 'Nexus API');
        if ($response->is_error()) {
            $data=$response->get_data();
            $response->set_data(['error'=>$data['code']??'nexus_error','message'=>$data['message']??'Nexus request failed.']);
        }
    }
    return $response;
}, 10, 3);

add_filter('rest_pre_serve_request', function ($served, $response, $request) {
    if ($response instanceof Nexus_Download_Response) {
        try { readfile($response->file); } finally { unlink($response->file); }
        return true;
    }
    return $served;
}, 10, 3);
