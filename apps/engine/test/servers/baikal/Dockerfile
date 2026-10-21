# Baikal 0.10.1 (sabre/dav 4) on nginx for the engine integration harness; see
# harness/servers/dav.py.
FROM ckulka/baikal:0.10.1-nginx
COPY baikal.yaml /var/www/baikal/config/baikal.yaml
COPY seed.sql /tmp/seed.sql
COPY default.conf.template /etc/nginx/templates/default.conf.template
RUN rm -f /etc/nginx/conf.d/default.conf \
 && mkdir -p /var/www/baikal/Specific/db \
 && sqlite3 /var/www/baikal/Specific/db/db.sqlite < /var/www/baikal/Core/Resources/Db/SQLite/db.sql \
 && sqlite3 /var/www/baikal/Specific/db/db.sqlite < /tmp/seed.sql \
 && rm /tmp/seed.sql \
 && chown -R nginx:nginx /var/www/baikal
# nginx listens on the port the harness publishes, so the absolute .well-known redirect
# names a port the engine can reach.
ENV BAIKAL_PORT=80
