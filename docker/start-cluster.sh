#!/usr/bin/env bash
# Start the NoriKV 3-node Docker cluster

set -e

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"

echo "🚀 Starting NoriKV cluster..."
echo ""

cd "$REPO_ROOT"

# Build if needed
if [ "$1" = "--build" ] || [ "$1" = "-b" ]; then
    echo "📦 Building Docker image..."
    docker compose build
    echo ""
fi

# Start the cluster
echo "🔧 Starting 3-node cluster..."
docker compose up -d

echo ""
echo "⏳ Waiting for nodes to start..."
sleep 5

echo ""
echo "✅ Cluster started successfully!"
echo ""
echo "📊 Node endpoints:"
echo "  - node1: localhost:7447 (gRPC), localhost:9090 (metrics)"
echo "  - node2: localhost:7448 (gRPC), localhost:9091 (metrics)"
echo "  - node3: localhost:7449 (gRPC), localhost:9092 (metrics)"
echo ""
echo "📝 View logs:"
echo "  docker compose logs -f"
echo ""
echo "🛑 Stop cluster:"
echo "  docker compose down"
echo ""
