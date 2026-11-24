# syntax=docker/dockerfile:1
# RepoHub in Docker: builds the web app (Rust → WebAssembly) and repohub-server, then runs the
# server, which serves the app on port 8080 and proxies the GitHub API with your token.
# Usage: see README.md ("Docker").

FROM rust:1-bookworm AS build
WORKDIR /src
# The toolchain version is pinned by rust-toolchain.toml; rustup installs it on first use.
# wasm-opt shrinks the .wasm; brotli and gzip make the copies the server sends to browsers.
COPY rust-toolchain.toml ./
COPY scripts/install-wasm-opt.sh scripts/
RUN rustup target add wasm32-unknown-unknown \
 && cargo install wasm-bindgen-cli --version 0.2.129 --locked \
 && scripts/install-wasm-opt.sh /usr/local/bin \
 && apt-get update && apt-get install -y --no-install-recommends brotli && rm -rf /var/lib/apt/lists/*
COPY . .
# The web app is built with the `web` profile (Cargo.toml), the server with its own.
RUN --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,target=/usr/local/cargo/git \
    --mount=type=cache,target=/src/target \
    --mount=type=cache,target=/src/server/target \
    scripts/web-build.sh /out/web --precompress \
 && cargo build --release --manifest-path server/Cargo.toml \
 && cp server/target/release/repohub-server /out/ \
 && mkdir -p /out/data

FROM gcr.io/distroless/cc-debian12:nonroot
# CA certificates for HTTPS to GitHub, taken from the build image (no package manager needed).
COPY --from=build /etc/ssl/certs/ca-certificates.crt /etc/ssl/certs/ca-certificates.crt
COPY --from=build /out/web /app/web
COPY --from=build /out/repohub-server /app/repohub-server
# Notes (drawings) live here; compose.yaml mounts a volume so they survive container updates.
COPY --from=build --chown=65534:65534 /out/data /data
ENV REPOHUB_ADDR=0.0.0.0:8080 \
    WEB_DIR=/app/web \
    DATA_DIR=/data
VOLUME /data
EXPOSE 8080
USER 65534:65534
CMD ["/app/repohub-server"]
