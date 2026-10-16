# Build from the repo root (tern-backend is a member of the root Cargo workspace):
#   docker build -f apps/backend/Dockerfile -t tern-backend .
# Dockerfile.dockerignore keeps the context to the workspace sources.
# The image binds 0.0.0.0:5101 (loopback is unreachable from a proxy container) and keeps all
# state in /data. Set TERN_PUBLIC_URL to the https URL clients and recipients use.

FROM rust:1.92-slim-bookworm AS build
ARG CARGO_BUILD_JOBS
WORKDIR /src
COPY Cargo.toml Cargo.lock ./
# Cargo reads every member's manifest; only tern-backend and ms-wire are compiled.
COPY crates ./crates
COPY apps/engine ./apps/engine
COPY apps/backend ./apps/backend
RUN cargo build --release --locked -p tern-backend --bin tern-backend \
 && mkdir -p /data

FROM gcr.io/distroless/cc-debian12:nonroot
COPY --from=build /src/target/release/tern-backend /usr/local/bin/tern-backend
COPY --from=build --chown=nonroot:nonroot /data /data
ENV TERN_DATA_DIR=/data \
    TERN_BIND=0.0.0.0:5101
VOLUME /data
EXPOSE 5101
ENTRYPOINT ["/usr/local/bin/tern-backend"]
CMD ["serve"]
