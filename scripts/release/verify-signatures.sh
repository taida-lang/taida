#!/usr/bin/env bash
# C26B-007 Sub-phase 7.4 (SEC-011): verify a downloaded Taida release
# artefact against its Sigstore cosign bundle.
#
# Intended callers:
#   - `install.sh` (the public installer fetched from taida.dev)
#   - `taida install` when resolving a first-party addon
#   - local downstream users who want to verify a tarball they pulled
#     from the GitHub release page
#
# Usage:
#   verify-signatures.sh <artefact-path> [--repo owner/name] [--tag TAG]
#
# Environment:
#   COSIGN_IDENTITY_REGEXP
#     Override the accepted certificate identity pattern.
#     Default: `^https://github.com/<REPO>/`.
#   COSIGN_OIDC_ISSUER
#     Override the expected OIDC issuer.
#     Default: `https://token.actions.githubusercontent.com`.
#
# Exit codes:
#   0   signature matches; caller may trust the artefact
#   2   missing cosign binary (install hint printed)
#   3   bundle file not found alongside artefact
#   4   cosign verify-blob rejected the signature (critical — stop)
#
# Safety: this script ALWAYS uses a regex-pinned certificate identity
# and a fixed OIDC issuer. A signature made by a different repo /
# identity will fail. Do not widen the regex at call sites without
# review.

set -euo pipefail

usage() {
    cat <<EOF
Usage: $0 <artefact-path> [--repo <owner/name>] [--tag <TAG>]

Verifies <artefact-path> against <artefact-path>.cosign.bundle via
cosign verify-blob. See https://docs.sigstore.dev/ for background on
Sigstore keyless signing.

Arguments:
  <artefact-path>   Path to the downloaded binary / archive.
  --repo            GitHub repository the artefact was published from.
                    Default: taida-lang/taida.
  --tag             Release tag name (informational; not used by the
                    verification step itself).
EOF
}

if [ $# -lt 1 ]; then
    usage
    exit 1
fi

ARTEFACT="$1"
shift || true
REPO="taida-lang/taida"
TAG=""
while [ $# -gt 0 ]; do
    case "$1" in
        --repo)
            REPO="$2"
            shift 2
            ;;
        --tag)
            TAG="$2"
            shift 2
            ;;
        -h|--help)
            usage
            exit 0
            ;;
        *)
            echo "error: unknown argument: $1" >&2
            usage
            exit 1
            ;;
    esac
done

if ! command -v cosign >/dev/null 2>&1; then
    cat >&2 <<EOF
error: \`cosign\` binary not found on PATH.

Install it via one of:
    # macOS / Linux
    brew install cosign
    # or direct download
    curl -sSL -o /usr/local/bin/cosign \\
        https://github.com/sigstore/cosign/releases/latest/download/cosign-linux-amd64
    chmod +x /usr/local/bin/cosign
EOF
    exit 2
fi

if [ ! -f "${ARTEFACT}" ]; then
    echo "error: artefact not found: ${ARTEFACT}" >&2
    exit 1
fi

BUNDLE="${ARTEFACT}.cosign.bundle"
if [ ! -f "${BUNDLE}" ]; then
    cat >&2 <<EOF
error: cosign bundle missing: ${BUNDLE}

The Sigstore bundle is published alongside every official Taida release
asset under the same filename with the suffix \`.cosign.bundle\`. Fetch
it from the same GitHub Release that provided ${ARTEFACT}.
EOF
    exit 3
fi

COSIGN_IDENTITY_REGEXP="${COSIGN_IDENTITY_REGEXP:-^https://github.com/${REPO}/}"
COSIGN_OIDC_ISSUER="${COSIGN_OIDC_ISSUER:-https://token.actions.githubusercontent.com}"

echo "SEC-011 verify: ${ARTEFACT}"
echo "  bundle:        ${BUNDLE}"
echo "  identity re:   ${COSIGN_IDENTITY_REGEXP}"
echo "  OIDC issuer:   ${COSIGN_OIDC_ISSUER}"
if [ -n "${TAG}" ]; then
    echo "  tag (info):    ${TAG}"
fi

if cosign verify-blob \
        --bundle "${BUNDLE}" \
        --certificate-identity-regexp "${COSIGN_IDENTITY_REGEXP}" \
        --certificate-oidc-issuer "${COSIGN_OIDC_ISSUER}" \
        "${ARTEFACT}"
then
    echo "SEC-011 verify: OK"
    exit 0
fi

echo "::error::SEC-011 verify: cosign verify-blob rejected ${ARTEFACT}" >&2
exit 4
