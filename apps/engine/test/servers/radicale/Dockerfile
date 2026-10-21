# Radicale 3 for the engine integration harness; see harness/servers/dav.py.
FROM python:3.12-alpine
RUN pip install --no-cache-dir radicale==3.8.3
COPY config /etc/radicale/config
COPY users /etc/radicale/users
COPY seed/ /data/collections/collection-root/
EXPOSE 5232
CMD ["python", "-m", "radicale", "--config", "/etc/radicale/config"]
