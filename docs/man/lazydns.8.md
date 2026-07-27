---
title: lazydns
section: 8
version: "0.2.50"
date: 2025-12-28
manual: "lazydns manual"
---

## NAME

lazydns \- system administration and runtime operations

## SYNOPSIS

```sh
# systemd
systemctl start lazydns
systemctl stop lazydns
systemctl restart lazydns

# journalctl logs
journalctl -u lazydns -f
```

## DESCRIPTION

This page documents recommended system administration tasks for running
`lazydns` in production, including service unit examples, logging, and common
operational workflows.

## SYSTEMD UNIT (EXAMPLE)

Place the following unit at `/etc/systemd/system/lazydns.service` for
systems using systemd (adjust paths and user/group as appropriate):

```ini
[Unit]
Description=LazyDNS DNS server
After=network.target

[Service]
Type=simple
User=lazydns
Group=lazydns
ExecStart=/usr/local/bin/lazydns --config /etc/lazydns/config.yaml
Restart=on-failure

[Install]
WantedBy=multi-user.target
```

Reload the unit and start the service:

```sh
sudo systemctl daemon-reload
sudo systemctl enable --now lazydns
```

## LOGS AND TROUBLESHOOTING

Use `journalctl -u lazydns -f` to follow runtime logs when running under
systemd. If lazydns is packaged and configured to log to files, check the
configured log file path (example: `/var/log/lazydns/lazydns.log`).

When debugging startup issues:

- ensure the configuration is valid YAML and contains required sections
- run `lazydns --config /path/to/config.yaml` in foreground to observe
  immediate errors
- check permissions for any TLS keys, sockets or privileged ports

## SAFE UPGRADE AND ROLLBACK

When upgrading packages:

- test new versions in a staging environment with production-like configs
- verify Prometheus metrics and admin endpoints respond before routing
  traffic to updated instances
- keep prior version packages available for quick rollback

## ADMIN API

`lazydns` exposes an admin HTTP API (see your configuration `admin.addr`) for
runtime operations (reload, stats). Use the admin endpoint where supported
instead of signalling directly when integrated with orchestration systems.

## FILES

- systemd unit: `/etc/systemd/system/lazydns.service` (example)
- config: `/etc/lazydns/config.yaml`
- logs: configured path in `logging.file` or systemd journal

## SEE ALSO

`lazydns.1`(1), `lazydns.5`(5)
