#!/usr/bin/env bash
set -euo pipefail

CERTS_DIR="certs"
mkdir -p "$CERTS_DIR"

echo "==> Generating CA key and certificate"
openssl genrsa -out "$CERTS_DIR/ca.key" 4096
openssl req -new -x509 -days 3650 -key "$CERTS_DIR/ca.key" \
    -out "$CERTS_DIR/ca.crt" \
    -subj "/CN=k8s-sentinel-ca"

echo "==> Generating server key and CSR"
openssl genrsa -out "$CERTS_DIR/tls.key" 4096

openssl req -new -key "$CERTS_DIR/tls.key" \
    -out "$CERTS_DIR/tls.csr" \
    -subj "/CN=k8s-sentinel" \
    -addext "subjectAltName=DNS:localhost,IP:127.0.0.1,DNS:k8s-sentinel.sentinel.svc,DNS:k8s-sentinel.sentinel.svc.cluster.local"

echo "==> Signing server certificate with CA"
openssl x509 -req -days 365 \
    -in "$CERTS_DIR/tls.csr" \
    -CA "$CERTS_DIR/ca.crt" \
    -CAkey "$CERTS_DIR/ca.key" \
    -CAcreateserial \
    -out "$CERTS_DIR/tls.crt" \
    -copy_extensions copyall

# Clean up intermediary files
rm -f "$CERTS_DIR/tls.csr" "$CERTS_DIR/ca.srl" "$CERTS_DIR/ca.key"

echo "==> Certificates generated:"
echo "    $CERTS_DIR/ca.crt   (CA certificate)"
echo "    $CERTS_DIR/tls.crt  (Server certificate)"
echo "    $CERTS_DIR/tls.key  (Server private key)"
