# SPDX-License-Identifier: AGPL-3.0-only
#
# Multi-stage build for mvs-manager. Build:
#   docker build -t mvs-manager .
# Run against a mounted project:
#   docker run --rm -v "$PWD:/workspace" -w /workspace mvs-manager lint

FROM rust:1-bookworm AS builder
WORKDIR /build

# Layer dependency compilation separately from source so an edit to src/
# doesn't invalidate the (much slower) dependency-only build layer.
COPY Cargo.toml Cargo.lock ./
COPY crates/mvs-core/Cargo.toml crates/mvs-core/Cargo.toml
COPY crates/mvs-crawler/Cargo.toml crates/mvs-crawler/Cargo.toml
COPY crates/mvs-wasm/Cargo.toml crates/mvs-wasm/Cargo.toml
RUN mkdir -p src crates/mvs-core/src crates/mvs-crawler/src crates/mvs-wasm/src \
    && echo 'fn main() {}' > src/main.rs \
    && echo '' > src/lib.rs \
    && echo '' > crates/mvs-core/src/lib.rs \
    && echo '' > crates/mvs-crawler/src/lib.rs \
    && echo '' > crates/mvs-wasm/src/lib.rs \
    && cargo build --release --locked || true

COPY . .
# Touch so Cargo doesn't reuse the placeholder objects from the warm-up build.
RUN find src crates -name '*.rs' -exec touch {} + \
    && cargo build --release --locked --bin mvs-manager

FROM debian:bookworm-slim AS runtime
RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates git \
    && rm -rf /var/lib/apt/lists/*
COPY --from=builder /build/target/release/mvs-manager /usr/local/bin/mvs-manager
ENV MVS_NO_UPDATE_CHECK=1
ENTRYPOINT ["mvs-manager"]
CMD ["--help"]
