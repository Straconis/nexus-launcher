<?php
if (!defined('ABSPATH')) { exit; }
final class Nexus_Store {
    public static function table(): string { global $wpdb; return $wpdb->prefix . 'nexus_records'; }
    public static function install(): void {
        global $wpdb;
        require_once ABSPATH . 'wp-admin/includes/upgrade.php';
        $table = self::table(); $charset = $wpdb->get_charset_collate();
        dbDelta("CREATE TABLE $table (
            record_kind varchar(24) NOT NULL,
            record_id varchar(191) NOT NULL,
            body longtext NOT NULL,
            expires bigint NOT NULL DEFAULT 0,
            PRIMARY KEY  (record_kind,record_id)
        ) $charset;");
        add_option('nexus_categories', ['Hardware', 'Hardware / Electronics', 'Hardware / 3D Printing', 'Hardware / Enclosures', 'Hardware / Prototypes'], '', false);
        add_option('nexus_project_types', ['Community', 'Podcast', 'Twitch stream'], '', false);
        add_option('nexus_status_stamps', [
            ['id'=>'released','name'=>'Released','color'=>'#52d69a'],
            ['id'=>'in_development','name'=>'In development','color'=>'#f2bc56'],
            ['id'=>'seeking_funding','name'=>'Seeking funding for prototype','color'=>'#5dc8ed'],
        ], '', false);
    }
    public static function put(string $kind, string $id, array $body, int $expires=0): void {
        global $wpdb;
        if (false === $wpdb->replace(self::table(), ['record_kind'=>$kind,'record_id'=>$id,'body'=>wp_json_encode($body),'expires'=>$expires], ['%s','%s','%s','%d'])) {
            throw new RuntimeException('Could not save Nexus data.');
        }
    }
    public static function get(string $kind, string $id, bool $expired=false): ?array {
        global $wpdb;
        $row = $wpdb->get_row($wpdb->prepare('SELECT body,expires FROM '.self::table().' WHERE record_kind=%s AND record_id=%s', $kind, $id), ARRAY_A);
        if (!$row || (!$expired && (int)$row['expires'] > 0 && (int)$row['expires'] <= time())) { return null; }
        return json_decode($row['body'], true);
    }
    public static function delete(string $kind, string $id): void {
        global $wpdb; $wpdb->delete(self::table(), ['record_kind'=>$kind,'record_id'=>$id], ['%s','%s']);
    }
    public static function all(string $kind): array {
        global $wpdb;
        return array_map(fn($value)=>json_decode($value,true), $wpdb->get_col($wpdb->prepare('SELECT body FROM '.self::table().' WHERE record_kind=%s ORDER BY record_id', $kind)));
    }
    public static function claim_flow(string $id, array $before, array $after, int $expires): bool {
        global $wpdb;
        return 1 === $wpdb->query($wpdb->prepare('UPDATE '.self::table().' SET body=%s WHERE record_kind=%s AND record_id=%s AND body=%s AND expires>%d', wp_json_encode($after), 'flow', $id, wp_json_encode($before), time()));
    }
    public static function cleanup(): void {
        global $wpdb;
        $wpdb->query($wpdb->prepare('DELETE FROM '.self::table().' WHERE expires>0 AND expires<%d AND record_kind IN (%s,%s)', time(), 'session', 'flow'));
    }
    public static function setting(string $name, string $default=''): string {
        $constant = 'NEXUS_' . strtoupper($name);
        return defined($constant) ? (string)constant($constant) : (string)get_option('nexus_'.$name, $default);
    }
}
