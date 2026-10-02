#!/usr/bin/env bash
# Start two local WebDAV servers for the Linux e2e suite (run as root):
#   Apache mod_dav  127.0.0.1:8081  — honours If-Match / If-Unmodified-Since (but see README:
#                                      IUS is compared against sub-second mtime → spurious 412)
#   nginx dav       127.0.0.1:8082  — ignores PUT preconditions, no getetag in PROPFIND
# Both use basic auth e2e / e2e-pass.
#
# Packages (Ubuntu 24.04): apache2 apache2-utils nginx libnginx-mod-http-dav-ext
set -euo pipefail

ROOT="${MT_E2E_DAV_ROOT:-/srv/mtodo-e2e}"
mkdir -p "$ROOT/apache/root" "$ROOT/apache/run" "$ROOT/nginx/root" "$ROOT/nginx/body"
htpasswd -bc "$ROOT/apache/htpasswd" e2e e2e-pass >/dev/null 2>&1
cp "$ROOT/apache/htpasswd" "$ROOT/nginx/htpasswd"
chown -R www-data:www-data "$ROOT/apache"
chmod 755 "$(dirname "$ROOT")" "$ROOT"

cat > "$ROOT/apache/httpd.conf" <<EOF
ServerRoot "$ROOT/apache"
ServerName localhost
Listen 127.0.0.1:8081
PidFile $ROOT/apache/run/httpd.pid
LoadModule mpm_event_module /usr/lib/apache2/modules/mod_mpm_event.so
LoadModule authn_core_module /usr/lib/apache2/modules/mod_authn_core.so
LoadModule authn_file_module /usr/lib/apache2/modules/mod_authn_file.so
LoadModule authz_core_module /usr/lib/apache2/modules/mod_authz_core.so
LoadModule authz_user_module /usr/lib/apache2/modules/mod_authz_user.so
LoadModule auth_basic_module /usr/lib/apache2/modules/mod_auth_basic.so
LoadModule dav_module /usr/lib/apache2/modules/mod_dav.so
LoadModule dav_fs_module /usr/lib/apache2/modules/mod_dav_fs.so
User www-data
Group www-data
ErrorLog $ROOT/apache/run/error.log
LogFormat "%h %r %>s" short
CustomLog $ROOT/apache/run/access.log short
DAVLockDB $ROOT/apache/run/davlock
DocumentRoot $ROOT/apache/root
<Directory />
  AllowOverride None
</Directory>
<Directory $ROOT/apache/root>
  AllowOverride None
  Dav On
  AuthType Basic
  AuthName "dav"
  AuthUserFile $ROOT/apache/htpasswd
  Require valid-user
</Directory>
EOF

cat > "$ROOT/nginx/nginx.conf" <<EOF
load_module /usr/lib/nginx/modules/ngx_http_dav_ext_module.so;
user root;
worker_processes 1;
pid $ROOT/nginx/nginx.pid;
error_log $ROOT/nginx/error.log;
events {}
http {
  access_log $ROOT/nginx/access.log;
  client_body_temp_path $ROOT/nginx/body;
  client_max_body_size 100m;
  server {
    listen 127.0.0.1:8082;
    root $ROOT/nginx/root;
    location / {
      dav_methods PUT DELETE MKCOL COPY MOVE;
      dav_ext_methods PROPFIND OPTIONS;
      create_full_put_path on;
      dav_access user:rw group:rw all:r;
      auth_basic "dav";
      auth_basic_user_file $ROOT/nginx/htpasswd;
    }
  }
}
EOF

apache2 -f "$ROOT/apache/httpd.conf" -k stop 2>/dev/null || true
nginx -c "$ROOT/nginx/nginx.conf" -s stop 2>/dev/null || true
sleep 1
apache2 -f "$ROOT/apache/httpd.conf" -k start
nginx -c "$ROOT/nginx/nginx.conf"
sleep 1
for p in 8081 8082; do
  code=$(curl -s -o /dev/null -w "%{http_code}" -u e2e:e2e-pass -T /etc/hostname "http://127.0.0.1:$p/.probe")
  echo "WebDAV on :$p -> PUT $code"
done
