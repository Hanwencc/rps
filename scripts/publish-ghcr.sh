#!/usr/bin/env sh
set -eu

usage() {
  cat <<'EOF'
Usage:
  scripts/publish-ghcr.sh <version>

Examples:
  scripts/publish-ghcr.sh v0.1.0
  IMAGE_NAMESPACE=ghcr.io/hanwencc PUSH_LATEST=false scripts/publish-ghcr.sh v0.1.0
  RPS_PLATFORMS=linux/amd64,linux/arm64 scripts/publish-ghcr.sh v0.1.0

Environment:
  IMAGE_NAMESPACE  Registry namespace. Default: ghcr.io/hanwencc
  PUSH_LATEST      Also push latest tags. Default: true
  RPS_PLATFORMS    When set, use docker buildx --platform and push directly.
EOF
}

if [ "${1:-}" = "-h" ] || [ "${1:-}" = "--help" ]; then
  usage
  exit 0
fi

VERSION="${1:-}"
if [ -z "$VERSION" ]; then
  usage
  exit 1
fi

case "$VERSION" in
  -*|*' '*|*':'*)
    echo "Invalid version tag: $VERSION" >&2
    exit 1
    ;;
esac

if ! command -v docker >/dev/null 2>&1; then
  echo "docker is required" >&2
  exit 1
fi

SCRIPT_DIR=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
REPO_ROOT=$(CDPATH= cd -- "$SCRIPT_DIR/.." && pwd)
IMAGE_NAMESPACE="${IMAGE_NAMESPACE:-ghcr.io/hanwencc}"
PUSH_LATEST="${PUSH_LATEST:-true}"
RPS_PLATFORMS="${RPS_PLATFORMS:-}"

CONTROLLER_IMAGE="$IMAGE_NAMESPACE/rps-controller"
AGENT_IMAGE="$IMAGE_NAMESPACE/rps-agent"

build_and_push() {
  name="$1"
  dockerfile="$2"
  image="$3"

  echo "==> Publishing $name as $image:$VERSION"

  if [ -n "$RPS_PLATFORMS" ]; then
    if [ "$PUSH_LATEST" = "true" ]; then
      docker buildx build \
        --platform "$RPS_PLATFORMS" \
        -f "$dockerfile" \
        -t "$image:$VERSION" \
        -t "$image:latest" \
        --push \
        "$REPO_ROOT"
    else
      docker buildx build \
        --platform "$RPS_PLATFORMS" \
        -f "$dockerfile" \
        -t "$image:$VERSION" \
        --push \
        "$REPO_ROOT"
    fi
    return
  fi

  docker build -f "$dockerfile" -t "$image:$VERSION" "$REPO_ROOT"
  docker push "$image:$VERSION"

  if [ "$PUSH_LATEST" = "true" ]; then
    docker tag "$image:$VERSION" "$image:latest"
    docker push "$image:latest"
  fi
}

build_and_push "controller" "$REPO_ROOT/docker/Dockerfile.controller" "$CONTROLLER_IMAGE"
build_and_push "agent" "$REPO_ROOT/docker/Dockerfile.agent" "$AGENT_IMAGE"

echo "==> Done"
echo "Published:"
echo "  $CONTROLLER_IMAGE:$VERSION"
echo "  $AGENT_IMAGE:$VERSION"
if [ "$PUSH_LATEST" = "true" ]; then
  echo "  $CONTROLLER_IMAGE:latest"
  echo "  $AGENT_IMAGE:latest"
fi
