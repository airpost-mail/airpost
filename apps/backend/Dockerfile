# Build from the repo root (tern-backend is a member of the root Cargo workspace):
#   docker build -f apps/backend/Dockerfile -t tern-backend .
# Dockerfile.dockerignore keeps the context to the workspace sources.
# The image binds 0.0.0.0:5101 (loopback is unreachable from a proxy container) and keeps all
# state in /data; /backups is there for scheduled backups (TERN_BACKUP_DIR). The health check is
# `tern-backend healthcheck`, since the image has no shell or curl. Set TERN_PUBLIC_URL to the
# https URL clients and recipients use.
# /usr/share/doc/tern-backend holds the licence (GPL-3.0-only), NOTICE.md, the crates'
# licences and SOURCE.txt. A published image must be built with SOURCE_URL (the repository URL)
# and SOURCE_TAG (a tag naming the commit built), which SOURCE.txt and the OCI labels point to.

FROM rust:1.92-slim-bookworm AS build
ARG CARGO_BUILD_JOBS
WORKDIR /src
# python3 is for third-party-notices.sh; cargo-about reads the crate sources cargo fetches.
RUN apt-get update -qq \
 && apt-get install -y -qq --no-install-recommends python3 >/dev/null \
 && rm -rf /var/lib/apt/lists/* \
 && cargo install --locked cargo-about --version 0.9.2 --features cli
COPY Cargo.toml Cargo.lock about.toml about.hbs LICENSE.md NOTICE.md ./
# Cargo reads every workspace member's manifest and targets, so every Rust app is copied (and
# listed in Dockerfile.dockerignore); only tern-backend and ms-wire are compiled.
COPY crates ./crates
COPY apps/engine ./apps/engine
COPY apps/cli ./apps/cli
COPY apps/gateway ./apps/gateway
COPY apps/mcp ./apps/mcp
COPY apps/backend ./apps/backend
RUN cargo build --release --locked -p tern-backend --bin tern-backend \
 && apps/engine/packaging/third-party-notices.sh /doc/THIRD-PARTY-NOTICES.txt apps/backend/Cargo.toml \
 && cp LICENSE.md NOTICE.md /doc/ \
 && mkdir -p /data /backups
ARG SOURCE_URL=
ARG SOURCE_TAG=
# GPLv3 6(d) and LGPL-2.1 6: directions to the source of tern-backend and of the Debian packages
# (glibc, libgcc, libstdc++, OpenSSL) in the distroless base layers.
RUN { \
  if [ -n "$SOURCE_URL" ] && [ -n "$SOURCE_TAG" ]; then \
    printf 'This is the tern-backend image, built from the tag %s.\n\n' "$SOURCE_TAG"; \
    printf 'tern-backend is licensed GPL-3.0-only. Its complete corresponding source is:\n\n'; \
    printf '  %s/tree/%s\n  %s/archive/refs/tags/%s.tar.gz\n\n' "$SOURCE_URL" "$SOURCE_TAG" "$SOURCE_URL" "$SOURCE_TAG"; \
  else \
    printf 'This image was built from an untagged tree and must not be published.\n\n'; \
  fi; \
  printf 'The base layers are gcr.io/distroless/cc-debian12, made of Debian 12 packages whose\n'; \
  printf 'licences are in /usr/share/doc/*/copyright. Their source is the matching Debian source\n'; \
  printf 'packages, at https://sources.debian.org and https://snapshot.debian.org.\n'; \
} > /doc/SOURCE.txt

FROM gcr.io/distroless/cc-debian12:nonroot
ARG SOURCE_URL=
ARG SOURCE_TAG=
ARG SOURCE_REVISION=
LABEL org.opencontainers.image.licenses="GPL-3.0-only" \
      org.opencontainers.image.source="$SOURCE_URL" \
      org.opencontainers.image.version="$SOURCE_TAG" \
      org.opencontainers.image.revision="$SOURCE_REVISION"
COPY --from=build /src/target/release/tern-backend /usr/local/bin/tern-backend
COPY --from=build /doc /usr/share/doc/tern-backend
COPY --from=build --chown=nonroot:nonroot /data /data
# Empty and owned by the server, so a volume mounted here (deploy/backend) is writable.
COPY --from=build --chown=nonroot:nonroot /backups /backups
ENV TERN_DATA_DIR=/data \
    TERN_BIND=0.0.0.0:5101
VOLUME /data
EXPOSE 5101
ENTRYPOINT ["/usr/local/bin/tern-backend"]
CMD ["serve"]
