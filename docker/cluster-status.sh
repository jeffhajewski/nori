#!/usr/bin/env bash
# Check the status of the NoriKV Docker cluster

set -e

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"

cd "$REPO_ROOT"

echo "📊 NoriKV Cluster Status"
echo "========================"
echo ""

# Check if cluster is running
if ! docker compose ps | grep -q "norikv-node"; then
    echo "❌ Cluster is not running."
    echo ""
    echo "Start the cluster with: ./docker/start-cluster.sh"
    exit 1
fi

# Show container status
echo "🐳 Container Status:"
docker compose ps
echo ""

# Show resource usage
echo "💻 Resource Usage:"
docker stats --no-stream --format "table {{.Name}}\t{{.CPUPerc}}\t{{.MemUsage}}" \
    norikv-node1 norikv-node2 norikv-node3 2>/dev/null || echo "  Unable to fetch stats"
echo ""

# Check connectivity
echo "🔗 Node Connectivity:"
for port in 7447 7448 7449; do
    if nc -z localhost $port 2>/dev/null; then
        echo "  ✓ localhost:$port (gRPC) - reachable"
    else
        echo "  ✗ localhost:$port (gRPC) - not reachable"
    fi
done
echo ""

# Check metrics endpoints
echo "📈 Metrics Endpoints:"
for port in 9090 9091 9092; do
    if curl -s -o /dev/null -w "%{http_code}" http://localhost:$port/metrics 2>/dev/null | grep -q "200"; then
        echo "  ✓ localhost:$port/metrics - accessible"
    else
        echo "  ✗ localhost:$port/metrics - not accessible"
    fi
done
echo ""

# Show recent errors
echo "⚠️  Recent Errors (last 10):"
docker compose logs --tail=100 2>&1 | grep -i "error\|panic\|fatal" | tail -10 || echo "  No recent errors"
echo ""

echo "💡 Commands:"
echo "  View logs:    docker compose logs -f"
echo "  Restart:      docker compose restart"
echo "  Stop:         ./docker/stop-cluster.sh"
echo ""
