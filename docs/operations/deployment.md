# Deployment Guide

Production deployment patterns for NoriKV on bare metal, Docker, and Kubernetes.

---

---

## Overview

This guide covers deploying NoriKV in various environments:

- **[Single-Node](#single-node-deployment)** - Development and testing
- **[3-Node Cluster (Bare Metal)](#3-node-cluster-bare-metal)** - Traditional servers
- **[Docker](#docker-deployment)** - Containerized deployment
- **[Kubernetes](#kubernetes-deployment)** - Cloud-native orchestration
- **[Cloud Providers](#cloud-provider-specific)** - AWS, GCP, Azure

---

## Prerequisites

### Hardware Requirements

**Minimum (Development):**
- **CPU:** 2 cores
- **RAM:** 4 GB
- **Disk:** 20 GB SSD

**Recommended (Production):**
- **CPU:** 8 cores (Intel Xeon or AMD EPYC)
- **RAM:** 32 GB
- **Disk:** 500 GB NVMe SSD (3000+ IOPS)
- **Network:** 1 Gbps

**Scaling guidelines:**

| Cluster Size | vCPU/node | RAM/node | Disk/node | Network |
|--------------|-----------|----------|-----------|---------|
| 1 node (dev) | 2-4 | 8 GB | 100 GB SSD | 100 Mbps |
| 3 nodes (small prod) | 4-8 | 16-32 GB | 500 GB NVMe | 1 Gbps |
| 9 nodes (medium prod) | 8-16 | 32-64 GB | 1 TB NVMe | 10 Gbps |
| 27 nodes (large prod) | 16-32 | 64-128 GB | 2 TB NVMe | 10 Gbps |

---

### Software Requirements

- **Operating System:** Linux (Ubuntu 22.04, RHEL 8+, Debian 11+)
- **Kernel:** 5.10+ (for modern async I/O)
- **systemd:** For service management
- **Docker:** 20.10+ (if using containers)
- **Kubernetes:** 1.25+ (if using K8s)

---

### Network Requirements

**Ports:**

| Port | Protocol | Purpose | Firewall Rule |
|------|----------|---------|---------------|
| 7447 | TCP | gRPC (client + Raft) | Open to clients and cluster |
| 8080 | TCP | HTTP (health/metrics) | Open to monitoring only |

**Bandwidth:**
- **Intra-cluster:** 1 Gbps minimum (10 Gbps recommended)
- **Client-to-cluster:** Depends on workload (1-10 Gbps)

**Latency:**
- **Same datacenter:** <1ms RTT
- **Multi-AZ:** <5ms RTT
- **Multi-region:** <50ms RTT (adjust Raft timeouts)

---

## Single-Node Deployment

Perfect for development, testing, and small datasets (<100GB).

### Installation

**Download binary:**

```bash
# Download latest release
wget https://github.com/norikv/norikv/releases/download/v0.1.0/norikv-server-linux-amd64

# Make executable
chmod +x norikv-server-linux-amd64
sudo mv norikv-server-linux-amd64 /usr/local/bin/norikv-server
```

**Or build from source:**

```bash
git clone https://github.com/norikv/norikv.git
cd norikv
cargo build --release -p norikv-server
sudo cp target/release/norikv-server /usr/local/bin/
```

---

### Configuration

**Create config file:**

```bash
sudo mkdir -p /etc/norikv
sudo vim /etc/norikv/config.yaml
```

**`/etc/norikv/config.yaml`:**

```yaml
node_id: "node0"
rpc_addr: "0.0.0.0:7447"
http_addr: "0.0.0.0:8080"
data_dir: "/var/lib/norikv"

cluster:
  seed_nodes: []  # Single-node mode
  total_shards: 128
  replication_factor: 1

telemetry:
  prometheus:
    enabled: true
```

---

### Create System User

```bash
# Create norikv user
sudo useradd -r -s /bin/false norikv

# Create data directory
sudo mkdir -p /var/lib/norikv
sudo chown norikv:norikv /var/lib/norikv
sudo chmod 750 /var/lib/norikv
```

---

### systemd Service

**Create service file:**

```bash
sudo vim /etc/systemd/system/norikv.service
```

**`/etc/systemd/system/norikv.service`:**

```ini
[Unit]
Description=NoriKV Server
Documentation=https://docs.norikv.io
After=network-online.target
Wants=network-online.target

[Service]
Type=simple
User=norikv
Group=norikv
ExecStart=/usr/local/bin/norikv-server --config /etc/norikv/config.yaml
Restart=on-failure
RestartSec=5s
LimitNOFILE=65536
StandardOutput=journal
StandardError=journal

# Security hardening
NoNewPrivileges=true
PrivateTmp=true
ProtectSystem=strict
ProtectHome=true
ReadWritePaths=/var/lib/norikv

[Install]
WantedBy=multi-user.target
```

---

### Start Service

```bash
# Reload systemd
sudo systemctl daemon-reload

# Enable service (start on boot)
sudo systemctl enable norikv

# Start service
sudo systemctl start norikv

# Check status
sudo systemctl status norikv
```

---

### Verify Deployment

**Check health:**

```bash
curl http://localhost:8080/health/quick
# Expected: OK

curl http://localhost:8080/health | jq
# Expected: {"status": "healthy", ...}
```

**Check metrics:**

```bash
curl http://localhost:8080/metrics | grep kv_requests_total
```

**Test with client:**

```bash
# Install grpcurl
brew install grpcurl  # or apt-get install grpcurl

# Test PUT
grpcurl -plaintext localhost:7447 norikv.Kv/Put \
  -d '{"key":"dGVzdA==","value":"dmFsdWU="}'

# Test GET
grpcurl -plaintext localhost:7447 norikv.Kv/Get \
  -d '{"key":"dGVzdA=="}'
```

---

### Logs

**View logs:**

```bash
# Tail logs
sudo journalctl -u norikv -f

# Last 100 lines
sudo journalctl -u norikv -n 100

# Since yesterday
sudo journalctl -u norikv --since yesterday

# Filter by ERROR
sudo journalctl -u norikv | grep ERROR
```

**Log format (JSON):**

```json
{
  "timestamp": "2024-01-15T10:30:00.123Z",
  "level": "INFO",
  "target": "norikv_server::node",
  "message": "Starting node",
  "fields": {
    "node_id": "node0"
  }
}
```

---

## 3-Node Cluster (Bare Metal)

Production deployment with fault tolerance and high availability.

### Architecture

```
┌─────────────────┐   ┌─────────────────┐   ┌─────────────────┐
│   Node 0        │   │   Node 1        │   │   Node 2        │
│   10.0.1.10     │   │   10.0.1.11     │   │   10.0.1.12     │
│                 │   │                 │   │                 │
│   RF=3          │   │   RF=3          │   │   RF=3          │
│   Shards:       │   │   Shards:       │   │   Shards:       │
│   0-341         │   │   0-341         │   │   0-341         │
│   (leader:      │   │   (leader:      │   │   (leader:      │
│    0-341)       │   │    342-682)     │   │    683-1023)    │
└─────────────────┘   └─────────────────┘   └─────────────────┘
         │                     │                     │
         └─────────────────────┴─────────────────────┘
                       Raft + SWIM gossip
```

---

### Node 0 Configuration

**`/etc/norikv/config.yaml`:**

```yaml
node_id: "node0"
rpc_addr: "10.0.1.10:7447"
http_addr: "10.0.1.10:8080"
data_dir: "/var/lib/norikv"

cluster:
  seed_nodes:
    - "10.0.1.10:7447"  # self
    - "10.0.1.11:7447"  # node1
    - "10.0.1.12:7447"  # node2
  total_shards: 1024
  replication_factor: 3

telemetry:
  prometheus:
    enabled: true
```

**Node 1 and Node 2:** Same config, change `node_id` and `rpc_addr`/`http_addr`

---

### Bootstrap Procedure

**1. Start Node 0 (first node):**

```bash
# On node0 (10.0.1.10)
sudo systemctl start norikv
sudo journalctl -u norikv -f
# Wait for "Node started successfully"
```

**2. Start Node 1:**

```bash
# On node1 (10.0.1.11)
sudo systemctl start norikv
# Logs should show: "Joining cluster via seed: 10.0.1.10:7447"
# Logs should show: "Member joined: node0 at 10.0.1.10:7447"
```

**3. Start Node 2:**

```bash
# On node2 (10.0.1.12)
sudo systemctl start norikv
# Cluster now has 3 nodes
```

**4. Verify cluster formation:**

```bash
# On any node
curl http://10.0.1.10:8080/health | jq '.nodes | length'
# Expected: 3

# Check SWIM cluster size
curl http://10.0.1.10:8080/metrics | grep swim_cluster_size
# Expected: swim_cluster_size 3
```

---

### Verify Replication

**Test write on node0, read from node1:**

```bash
# Write to node0
grpcurl -plaintext 10.0.1.10:7447 norikv.Kv/Put \
  -d '{"key":"dGVzdC1yZXBs","value":"cmVwbGljYXRlZA=="}'

# Wait for replication (1-2 seconds)
sleep 2

# Read from node1 (should succeed)
grpcurl -plaintext 10.0.1.11:7447 norikv.Kv/Get \
  -d '{"key":"dGVzdC1yZXBs"}'
# Expected: {"value":"cmVwbGljYXRlZA=="}
```

---

### Load Balancer Setup

**HAProxy configuration:**

```haproxy
# /etc/haproxy/haproxy.cfg

global
    log /dev/log local0
    log /dev/log local1 notice
    chroot /var/lib/haproxy
    stats socket /run/haproxy/admin.sock mode 660 level admin
    stats timeout 30s
    user haproxy
    group haproxy
    daemon

defaults
    log     global
    mode    tcp
    option  tcplog
    option  dontlognull
    timeout connect 5000
    timeout client  50000
    timeout server  50000

frontend norikv_grpc
    bind *:7447
    default_backend norikv_servers

backend norikv_servers
    balance roundrobin
    option httpchk GET /health/quick
    http-check expect status 200
    server node0 10.0.1.10:7447 check port 8080 inter 5s fall 3 rise 2
    server node1 10.0.1.11:7447 check port 8080 inter 5s fall 3 rise 2
    server node2 10.0.1.12:7447 check port 8080 inter 5s fall 3 rise 2

frontend norikv_http
    bind *:8080
    default_backend norikv_http_servers

backend norikv_http_servers
    balance roundrobin
    option httpchk GET /health/quick
    http-check expect status 200
    server node0 10.0.1.10:8080 check inter 5s
    server node1 10.0.1.11:8080 check inter 5s
    server node2 10.0.1.12:8080 check inter 5s
```

**Start HAProxy:**

```bash
sudo systemctl restart haproxy
sudo systemctl status haproxy
```

**Test via load balancer:**

```bash
grpcurl -plaintext localhost:7447 norikv.Kv/Put \
  -d '{"key":"bGI=","value":"dGVzdA=="}'
```

---

## Docker Deployment

### Single-Node Docker

**Create docker-compose.yml:**

```yaml
version: '3.8'

services:
  norikv:
    image: norikv/norikv-server:latest
    container_name: norikv-server
    ports:
      - "7447:7447"  # gRPC
      - "8080:8080"  # HTTP
    volumes:
      - norikv-data:/data
    environment:
      NORIKV_NODE_ID: "docker-node0"
      NORIKV_RPC_ADDR: "0.0.0.0:7447"
      NORIKV_HTTP_ADDR: "0.0.0.0:8080"
      NORIKV_DATA_DIR: "/data"
      NORIKV_SEED_NODES: ""  # Single-node mode
    restart: unless-stopped
    healthcheck:
      test: ["CMD", "curl", "-f", "http://localhost:8080/health/quick"]
      interval: 10s
      timeout: 5s
      retries: 3
      start_period: 30s

volumes:
  norikv-data:
    driver: local
```

**Start:**

```bash
docker-compose up -d
docker-compose logs -f norikv
```

---

### 3-Node Docker Cluster

**`docker-compose.yml`:**

```yaml
version: '3.8'

services:
  norikv-node0:
    image: norikv/norikv-server:latest
    container_name: norikv-node0
    hostname: norikv-node0
    ports:
      - "7447:7447"
      - "8080:8080"
    volumes:
      - node0-data:/data
    environment:
      NORIKV_NODE_ID: "node0"
      NORIKV_RPC_ADDR: "0.0.0.0:7447"
      NORIKV_HTTP_ADDR: "0.0.0.0:8080"
      NORIKV_DATA_DIR: "/data"
      NORIKV_SEED_NODES: "norikv-node0:7447,norikv-node1:7447,norikv-node2:7447"
    networks:
      - norikv-cluster
    restart: unless-stopped

  norikv-node1:
    image: norikv/norikv-server:latest
    container_name: norikv-node1
    hostname: norikv-node1
    ports:
      - "7448:7447"
      - "8081:8080"
    volumes:
      - node1-data:/data
    environment:
      NORIKV_NODE_ID: "node1"
      NORIKV_RPC_ADDR: "0.0.0.0:7447"
      NORIKV_HTTP_ADDR: "0.0.0.0:8080"
      NORIKV_DATA_DIR: "/data"
      NORIKV_SEED_NODES: "norikv-node0:7447,norikv-node1:7447,norikv-node2:7447"
    networks:
      - norikv-cluster
    restart: unless-stopped
    depends_on:
      - norikv-node0

  norikv-node2:
    image: norikv/norikv-server:latest
    container_name: norikv-node2
    hostname: norikv-node2
    ports:
      - "7449:7447"
      - "8082:8080"
    volumes:
      - node2-data:/data
    environment:
      NORIKV_NODE_ID: "node2"
      NORIKV_RPC_ADDR: "0.0.0.0:7447"
      NORIKV_HTTP_ADDR: "0.0.0.0:8080"
      NORIKV_DATA_DIR: "/data"
      NORIKV_SEED_NODES: "norikv-node0:7447,norikv-node1:7447,norikv-node2:7447"
    networks:
      - norikv-cluster
    restart: unless-stopped
    depends_on:
      - norikv-node0

networks:
  norikv-cluster:
    driver: bridge

volumes:
  node0-data:
  node1-data:
  node2-data:
```

**Start cluster:**

```bash
docker-compose up -d
docker-compose ps
docker-compose logs -f
```

---

## Kubernetes Deployment

### Architecture

```
┌────────────────────────────────────────────────┐
│  StatefulSet: norikv                           │
│  Replicas: 3                                   │
├────────────────────────────────────────────────┤
│  ┌─────────────┐  ┌─────────────┐  ┌─────────┐│
│  │ norikv-0    │  │ norikv-1    │  │ norikv-2││
│  │ PVC: 100Gi  │  │ PVC: 100Gi  │  │ PVC: ..  ││
│  └─────────────┘  └─────────────┘  └─────────┘│
└────────────────────────────────────────────────┘
              │
┌─────────────┴──────────────┐
│  Headless Service: norikv  │  (ClusterIP: None)
│  Port: 7447 (gRPC)         │
└────────────────────────────┘
```

---

### ConfigMap

**`norikv-configmap.yaml`:**

```yaml
apiVersion: v1
kind: ConfigMap
metadata:
  name: norikv-config
  namespace: default
data:
  config.yaml: |
    node_id: "$(POD_NAME)"
    rpc_addr: "0.0.0.0:7447"
    http_addr: "0.0.0.0:8080"
    data_dir: "/data"
    cluster:
      seed_nodes:
        - "norikv-0.norikv.default.svc.cluster.local:7447"
        - "norikv-1.norikv.default.svc.cluster.local:7447"
        - "norikv-2.norikv.default.svc.cluster.local:7447"
      total_shards: 1024
      replication_factor: 3
    telemetry:
      prometheus:
        enabled: true
```

---

### Headless Service

**`norikv-service.yaml`:**

```yaml
apiVersion: v1
kind: Service
metadata:
  name: norikv
  namespace: default
  labels:
    app: norikv
spec:
  clusterIP: None  # Headless service
  ports:
    - name: grpc
      port: 7447
      targetPort: 7447
      protocol: TCP
    - name: http
      port: 8080
      targetPort: 8080
      protocol: TCP
  selector:
    app: norikv
---
apiVersion: v1
kind: Service
metadata:
  name: norikv-lb
  namespace: default
  labels:
    app: norikv
spec:
  type: LoadBalancer
  ports:
    - name: grpc
      port: 7447
      targetPort: 7447
      protocol: TCP
  selector:
    app: norikv
```

---

### StatefulSet

**`norikv-statefulset.yaml`:**

```yaml
apiVersion: apps/v1
kind: StatefulSet
metadata:
  name: norikv
  namespace: default
spec:
  serviceName: norikv
  replicas: 3
  selector:
    matchLabels:
      app: norikv
  template:
    metadata:
      labels:
        app: norikv
      annotations:
        prometheus.io/scrape: "true"
        prometheus.io/port: "8080"
        prometheus.io/path: "/metrics"
    spec:
      containers:
      - name: norikv
        image: norikv/norikv-server:latest
        ports:
        - containerPort: 7447
          name: grpc
          protocol: TCP
        - containerPort: 8080
          name: http
          protocol: TCP
        env:
        - name: POD_NAME
          valueFrom:
            fieldRef:
              fieldPath: metadata.name
        - name: NORIKV_NODE_ID
          value: "$(POD_NAME)"
        volumeMounts:
        - name: data
          mountPath: /data
        - name: config
          mountPath: /etc/norikv
        livenessProbe:
          httpGet:
            path: /health/quick
            port: 8080
          initialDelaySeconds: 30
          periodSeconds: 10
          timeoutSeconds: 5
          failureThreshold: 3
        readinessProbe:
          httpGet:
            path: /health/quick
            port: 8080
          initialDelaySeconds: 10
          periodSeconds: 5
          timeoutSeconds: 3
          successThreshold: 1
        resources:
          requests:
            cpu: "2"
            memory: "8Gi"
          limits:
            cpu: "4"
            memory: "16Gi"
      volumes:
      - name: config
        configMap:
          name: norikv-config
  volumeClaimTemplates:
  - metadata:
      name: data
    spec:
      accessModes: [ "ReadWriteOnce" ]
      storageClassName: fast-ssd  # Adjust for your cloud
      resources:
        requests:
          storage: 100Gi
```

---

### Deploy to Kubernetes

**Apply manifests:**

```bash
kubectl apply -f norikv-configmap.yaml
kubectl apply -f norikv-service.yaml
kubectl apply -f norikv-statefulset.yaml
```

**Watch rollout:**

```bash
kubectl rollout status statefulset/norikv
kubectl get pods -l app=norikv -w
```

**Verify cluster:**

```bash
# Check pods
kubectl get pods -l app=norikv

# Check health
kubectl exec norikv-0 -- curl -s http://localhost:8080/health | jq

# Check cluster size
kubectl exec norikv-0 -- curl -s http://localhost:8080/metrics | grep swim_cluster_size
# Expected: swim_cluster_size 3
```

---

### Access from Outside K8s

**Via LoadBalancer:**

```bash
# Get LoadBalancer IP
kubectl get svc norikv-lb
# NAME        TYPE           CLUSTER-IP     EXTERNAL-IP      PORT(S)
# norikv-lb   LoadBalancer   10.100.200.1   35.123.45.67     7447:31234/TCP

# Test
grpcurl -plaintext 35.123.45.67:7447 norikv.Kv/Put \
  -d '{"key":"dGVzdA==","value":"dmFsdWU="}'
```

**Via Port-Forward (development):**

```bash
kubectl port-forward svc/norikv-lb 7447:7447
grpcurl -plaintext localhost:7447 norikv.Kv/Put \
  -d '{"key":"dGVzdA==","value":"dmFsdWU="}'
```

---

## Cloud Provider Specific

### AWS EKS

**Storage Class (gp3 SSD):**

```yaml
apiVersion: storage.k8s.io/v1
kind: StorageClass
metadata:
  name: fast-ssd
provisioner: ebs.csi.aws.com
parameters:
  type: gp3
  iops: "3000"
  throughput: "125"
volumeBindingMode: WaitForFirstConsumer
```

**Node affinity (i3en instances with NVMe):**

```yaml
affinity:
  nodeAffinity:
    requiredDuringSchedulingIgnoredDuringExecution:
      nodeSelectorTerms:
      - matchExpressions:
        - key: node.kubernetes.io/instance-type
          operator: In
          values:
          - i3en.xlarge
          - i3en.2xlarge
```

---

### GCP GKE

**Storage Class (pd-ssd):**

```yaml
apiVersion: storage.k8s.io/v1
kind: StorageClass
metadata:
  name: fast-ssd
provisioner: pd.csi.storage.gke.io
parameters:
  type: pd-ssd
  replication-type: regional-pd
volumeBindingMode: WaitForFirstConsumer
```

---

### Azure AKS

**Storage Class (Premium_LRS):**

```yaml
apiVersion: storage.k8s.io/v1
kind: StorageClass
metadata:
  name: fast-ssd
provisioner: disk.csi.azure.com
parameters:
  skuName: Premium_LRS
volumeBindingMode: WaitForFirstConsumer
```

---

## Production Checklist

### Pre-Deployment

- [ ] Size hardware (CPU, RAM, disk) based on workload
- [ ] Provision SSD/NVMe storage (not HDD)
- [ ] Configure firewall rules (ports 7447, 8080)
- [ ] Set up monitoring (Prometheus + Grafana)
- [ ] Configure log aggregation (ELK, Loki)
- [ ] Plan backup strategy (snapshots)

### Post-Deployment

- [ ] Verify cluster formation (3 nodes visible)
- [ ] Test replication (write on node0, read from node1)
- [ ] Load test (simulate production traffic)
- [ ] Test failure scenarios (kill node, network partition)
- [ ] Set up alerts (Prometheus Alertmanager)
- [ ] Document runbook (incident response)

---

## Next Steps

- **[Configuration Reference](configuration)** - Tune server settings
- **[Metrics](metrics)** - Set up monitoring
- **[Troubleshooting](troubleshooting)** - Common deployment issues
