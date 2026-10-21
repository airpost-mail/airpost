# syntax=docker/dockerfile:1
# The Tern server image: tern-engine, tern-gateway with the built web client, tern-backend,
# tern-mcp and tern, in one image that deploy/server/compose.yaml runs once per component.
# Build from the repo root:
#   docker build -f deploy/server/Dockerfile -t tern-server .
# Dockerfile.dockerignore next to this file keeps the context to the sources.
#
# Paths: the engine's profile is /profile (CONFIG_DIR_PATH, TERN_CONFIG_DIR), shared by every
# component that talks to the engine; the engine's key file is /keys/engine.key, a volume only
# the engine mounts; the backend keeps /data and /backups. All four are empty and owned by
# the image's user (nonroot, 65532), so volumes mounted there are writable. The image has no
# shell or curl: health checks are `tern status`, `tern-gateway healthcheck`,
# `tern-backend healthcheck` and `tern-mcp healthcheck`.
#
# /usr/share/doc/tern holds the licence (GPL-3.0-only), NOTICE.md, each binary's third-party
# notices and SOURCE.txt. A published image must be built with SOURCE_URL and SOURCE_TAG, as
# apps/backend/Dockerfile explains.

FROM node:22-bookworm-slim AS web
WORKDIR /web
COPY apps/web/package.json apps/web/package-lock.json ./
RUN npm ci --no-audit --no-fund --loglevel=error
COPY apps/web ./
RUN VITE_TERN_ENGINE=gateway npm run build

FROM rust:1.92-slim-bookworm AS build
ARG CARGO_BUILD_JOBS
WORKDIR /src
# python3 is for third-party-notices.sh; cargo-about reads the crate sources cargo fetches.
RUN apt-get update -qq \
 && apt-get install -y -qq --no-install-recommends python3 >/dev/null \
 && rm -rf /var/lib/apt/lists/* \
 && cargo install --locked cargo-about --version 0.9.2 --features cli
COPY Cargo.toml Cargo.lock about.toml about.hbs LICENSE.md NOTICE.md ./
COPY crates ./crates
COPY apps/engine ./apps/engine
COPY apps/cli ./apps/cli
COPY apps/gateway ./apps/gateway
COPY apps/mcp ./apps/mcp
COPY apps/backend ./apps/backend
COPY examples/service-plugin ./examples/service-plugin
# The release profile keeps line tables for crash reports from desktop builds; the image
# doesn't need them.
ENV CARGO_PROFILE_RELEASE_DEBUG=0 \
    CARGO_PROFILE_RELEASE_STRIP=symbols
# The cache mounts keep the registry and the target dir between builds; the binaries are
# copied out because a cache mount isn't part of the layer.
RUN --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,target=/src/target \
    cargo build --release --locked \
      -p tern-engine -p tern-gateway -p tern-backend -p tern-mcp -p tern \
 && mkdir -p /out/bin /doc \
 && for b in tern-engine tern-gateway tern-backend tern-mcp tern; do \
      install -m 755 "target/release/$b" /out/bin/; \
    done \
 && apps/engine/packaging/third-party-notices.sh /doc/tern-engine/THIRD-PARTY-NOTICES.txt apps/engine/Cargo.toml \
 && apps/engine/packaging/third-party-notices.sh /doc/tern-gateway/THIRD-PARTY-NOTICES.txt apps/gateway/Cargo.toml \
 && apps/engine/packaging/third-party-notices.sh /doc/tern-backend/THIRD-PARTY-NOTICES.txt apps/backend/Cargo.toml \
 && apps/engine/packaging/third-party-notices.sh /doc/tern-mcp/THIRD-PARTY-NOTICES.txt apps/mcp/Cargo.toml \
 && apps/engine/packaging/third-party-notices.sh /doc/tern/THIRD-PARTY-NOTICES.txt apps/cli/Cargo.toml \
 && cp LICENSE.md NOTICE.md /doc/ \
 && mkdir -p /skel/profile /skel/keys /skel/data /skel/backups \
 && chmod 700 /skel/profile /skel/keys /skel/data
ARG SOURCE_URL=
ARG SOURCE_TAG=
# GPLv3 6(d) and LGPL-2.1 6: directions to the source of Tern and of the Debian packages
# (glibc, libgcc, libstdc++, OpenSSL) in the distroless base layers. The web client's npm
# packages are listed in its own source (apps/web/package-lock.json).
RUN { \
  if [ -n "$SOURCE_URL" ] && [ -n "$SOURCE_TAG" ]; then \
    printf 'This is the tern-server image, built from the tag %s.\n\n' "$SOURCE_TAG"; \
    printf 'Tern is licensed GPL-3.0-only. Its complete corresponding source is:\n\n'; \
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
LABEL org.opencontainers.image.title="tern-server" \
      org.opencontainers.image.licenses="GPL-3.0-only" \
      org.opencontainers.image.source="$SOURCE_URL" \
      org.opencontainers.image.version="$SOURCE_TAG" \
      org.opencontainers.image.revision="$SOURCE_REVISION"
COPY --from=build /out/bin/ /usr/local/bin/
COPY --from=build /doc /usr/share/doc/tern
COPY --from=web /web/dist /usr/share/tern/web
COPY --from=build --chown=nonroot:nonroot /skel/profile /profile
COPY --from=build --chown=nonroot:nonroot /skel/keys /keys
COPY --from=build --chown=nonroot:nonroot /skel/data /data
COPY --from=build --chown=nonroot:nonroot /skel/backups /backups
# TERN_ENGINE_KEY_FILE picks the engine's file secret store (crates/tern-secrets): there is no
# keychain in a container. The engine creates the key on first use when the file is missing.
ENV CONFIG_DIR_PATH=/profile \
    TERN_CONFIG_DIR=/profile \
    TERN_ENGINE_KEY_FILE=/keys/engine.key \
    TERN_GATEWAY_WEB_ROOT=/usr/share/tern/web \
    TERN_GATEWAY_BIND=0.0.0.0:5180 \
    TERN_DATA_DIR=/data \
    TERN_BIND=0.0.0.0:5101
CMD ["/usr/local/bin/tern-engine", "--mode", "serve", "--detached"]
