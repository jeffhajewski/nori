#!/usr/bin/env bash
# Stop the NoriKV 3-node Docker cluster

set -e

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"

echo "🛑 Stopping NoriKV cluster..."
echo ""

cd "$REPO_ROOT"

# Check if we should remove volumes
if [ "$1" = "--clean" ] || [ "$1" = "-c" ]; then
    echo "🗑️  Stopping cluster and removing all data..."
    docker compose down -v
    echo ""
    echo "✅ Cluster stopped and all data removed."
else
    echo "📦 Stopping cluster (data preserved)..."
    docker compose down
    echo ""
    echo "✅ Cluster stopped. Data volumes preserved."
    echo ""
    echo "💡 To remove all data, use: $0 --clean"
fi

echo ""
