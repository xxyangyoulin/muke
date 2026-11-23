<?php
return [
    'id' => 'contract',
    'default_enabled' => true,
    'tables' => function (PDO $db): void { $GLOBALS['contract_migrations'] = ($GLOBALS['contract_migrations'] ?? 0) + 1; },
    'sync' => fn(PDO $db, array $config): array => [],
    'hooks' => ['post.saved' => function (array $payload): void { $GLOBALS['contract_events'][] = $payload; }],
    'export' => function (PDO $db, string $dest, array &$files, bool $enabled): array {
        if (!empty($GLOBALS['contract_export_fail'])) throw new RuntimeException('injected export failure');
        $GLOBALS['contract_exports'][] = $enabled;
        return ['items' => [], 'refs' => []];
    },
];
