<?php
// Install in the Hostinger subdomain folder directly below the existing WordPress root.
// This entry point exposes only the Nexus REST namespace.
$path = parse_url($_SERVER['REQUEST_URI'] ?? '/', PHP_URL_PATH);
if (!is_string($path) || !preg_match('~^/v1/([a-zA-Z0-9_/-]+)$~D', $path, $match)) {
    http_response_code(404);
    header('Content-Type: application/json');
    header('Cache-Control: no-store');
    echo '{"error":"not_found","message":"Use a Nexus /v1 endpoint."}';
    exit;
}
$_GET['rest_route'] = '/nexus/v1/' . $match[1];
define('WP_USE_THEMES', false);
require dirname(__DIR__) . '/wp-blog-header.php';
