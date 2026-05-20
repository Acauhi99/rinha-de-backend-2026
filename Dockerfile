FROM rust:1.85-slim-bookworm AS builder

WORKDIR /app

COPY Cargo.toml Cargo.lock* ./
COPY build-index/ build-index/
COPY api/ api/

RUN --mount=type=cache,target=/app/target \
    --mount=type=cache,target=/usr/local/cargo/registry \
    RUSTFLAGS="-C target-cpu=x86-64-v3" \
    cargo build --release -p api -p build-index && \
    cp target/release/api /api-bin && \
    cp target/release/build-index /build-index-bin

FROM debian:bookworm-slim AS index-builder

RUN apt-get update && apt-get install -y --no-install-recommends libgcc-s1 && rm -rf /var/lib/apt/lists/*

COPY --from=builder /build-index-bin /build-index
COPY resources/ /app/resources/
RUN /build-index /app/resources /app/index.bin

FROM gcr.io/distroless/cc-debian12:nonroot

COPY --from=builder /api-bin /api
COPY --from=index-builder /app/index.bin /index.bin

EXPOSE 3000

ENTRYPOINT ["/api", "/index.bin", "0.0.0.0:3000"]
