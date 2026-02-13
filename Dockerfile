# ── Stage 1: Builder ──────────────────────────────────────────────────────────
FROM rust:1.88-bookworm AS builder

# Docker sets TARGETARCH automatically (amd64 | arm64) with buildx / build
ARG TARGETARCH

# Map Docker arch to Rust target triple
RUN case "${TARGETARCH}" in \
      arm64) RUST_TARGET=aarch64-unknown-linux-gnu ;; \
      *)     RUST_TARGET=x86_64-unknown-linux-gnu  ;; \
    esac && \
    echo "$RUST_TARGET" > /tmp/rust-target && \
    rustup target add "$RUST_TARGET"

WORKDIR /app

# Cache dependency build: copy only manifests and build with a dummy main
COPY Cargo.toml Cargo.lock ./
RUN RUST_TARGET=$(cat /tmp/rust-target) && \
    mkdir src && echo 'fn main() {}' > src/main.rs && \
    cargo build --release --target "$RUST_TARGET" && \
    rm -rf src target/"$RUST_TARGET"/release/deps/k8s_sentinel*

# Copy real source and build the static binary
COPY src/ src/
RUN RUST_TARGET=$(cat /tmp/rust-target) && \
    RUSTFLAGS="-C target-feature=+crt-static" \
    cargo build --release --target "$RUST_TARGET" && \
    cp target/"$RUST_TARGET"/release/k8s-sentinel /app/k8s-sentinel

# ── Stage 2: Runtime (expected final image size: ~15-20 MB) ───────────────────
FROM gcr.io/distroless/static-debian12:nonroot

COPY --from=builder /app/k8s-sentinel /k8s-sentinel
COPY config/policies.yaml /etc/sentinel/policies.yaml

USER 65532:65532

EXPOSE 8443 9090

ENTRYPOINT ["/k8s-sentinel"]
