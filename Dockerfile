FROM rust:1-bookworm AS build
WORKDIR /build
COPY Cargo.toml Cargo.lock ./
COPY src ./src
RUN cargo build --release --locked

FROM debian:bookworm-slim
RUN apt-get update && apt-get install -y --no-install-recommends ca-certificates tesseract-ocr \
    && rm -rf /var/lib/apt/lists/*
COPY --from=build /build/target/release/cpass /usr/local/bin/cpass
WORKDIR /data
COPY config.yml /data/config.yml
ENTRYPOINT ["cpass"]
CMD ["--help"]
