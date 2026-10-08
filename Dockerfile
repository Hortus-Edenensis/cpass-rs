FROM rust:1.93-bookworm AS builder

WORKDIR /workspace
COPY Cargo.toml ./
COPY crates/cpass-core/Cargo.toml crates/cpass-core/Cargo.toml
COPY crates/cpass-cli/Cargo.toml crates/cpass-cli/Cargo.toml
RUN mkdir -p crates/cpass-core/src crates/cpass-cli/src && \
    printf 'pub fn bootstrap() {}\n' > crates/cpass-core/src/lib.rs && \
    printf 'fn main() {}\n' > crates/cpass-cli/src/main.rs && \
    cargo build --release -p cpass-cli && \
    rm -rf crates/cpass-core/src crates/cpass-cli/src

COPY . .
RUN cargo build --release -p cpass-cli

FROM debian:bookworm-slim AS runtime

RUN apt-get update && \
    apt-get install -y --no-install-recommends ca-certificates libsqlite3-0 && \
    rm -rf /var/lib/apt/lists/* && \
    useradd --create-home --uid 10001 cpass

WORKDIR /app
COPY --from=builder /workspace/target/release/cpass /usr/local/bin/cpass
COPY config.example.yml /app/config.example.yml
COPY docs/legacy-reference.md /app/legacy-reference.md

RUN mkdir -p /app/session /app/logs /app/export /app/faces && \
    chown -R cpass:cpass /app

USER cpass
ENTRYPOINT ["/usr/local/bin/cpass"]
CMD ["--help"]
