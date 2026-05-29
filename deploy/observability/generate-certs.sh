#!/usr/bin/env bash
set -euo pipefail

# Generate a self-signed CA + server certificate for the OTel collector.
#
# The server cert includes SAN entries for both `localhost` and `otel-collector`
# so it works whether Loong connects from the host or from inside Docker.
#
# Usage:  ./generate-certs.sh
# Output: certs/ca.pem       — CA cert, point OTEL_CA_CERT_FILE at this
#         certs/ca.key       — CA private key (keep safe)
#         certs/server.crt   — Collector server cert
#         certs/server.key   — Collector server private key

CERT_DIR="$(cd "$(dirname "$0")" && pwd)/certs"
DAYS=3650  # 10 years

mkdir -p "$CERT_DIR"

# --- CA ---
openssl genrsa -out "$CERT_DIR/ca.key" 2048

openssl req -x509 -new -nodes -key "$CERT_DIR/ca.key" \
  -sha256 -days "$DAYS" \
  -out "$CERT_DIR/ca.pem" \
  -subj "/C=CN/ST=Local/L=Dev/O=Loong Observability/CN=LoongDevCA"

# --- Server cert signed by CA ---
openssl genrsa -out "$CERT_DIR/server.key" 2048

openssl req -new -key "$CERT_DIR/server.key" \
  -out "$CERT_DIR/server.csr" \
  -subj "/C=CN/ST=Local/L=Dev/O=Loong Observability/CN=otel-collector"

# Write SAN config so the cert works for both docker-compose and host access
cat > "$CERT_DIR/server.ext" <<EOF
authorityKeyIdentifier=keyid,issuer
basicConstraints=CA:FALSE
keyUsage=digitalSignature,keyEncipherment
extendedKeyUsage=serverAuth
subjectAltName=@alt_names

[alt_names]
DNS.1=localhost
DNS.2=otel-collector
DNS.3=0.0.0.0
IP.1=127.0.0.1
EOF

openssl x509 -req -in "$CERT_DIR/server.csr" \
  -CA "$CERT_DIR/ca.pem" -CAkey "$CERT_DIR/ca.key" \
  -CAcreateserial -out "$CERT_DIR/server.crt" \
  -days "$DAYS" -sha256 \
  -extfile "$CERT_DIR/server.ext"

# Make keys world-readable so the collector container (non-root) can load them
chmod 644 "$CERT_DIR/server.key"

# Clean up temp files
rm -f "$CERT_DIR/server.csr" "$CERT_DIR/server.ext" "$CERT_DIR/ca.srl"

echo "Done! Generated in $CERT_DIR"
echo ""
echo "  Loong side:  export OTEL_CA_CERT_FILE=$CERT_DIR/ca.pem"
echo "  Collector:   certs/server.crt + certs/server.key"
