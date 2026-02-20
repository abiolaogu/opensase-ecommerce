# opensase-ecommerce

Self-hosted E-commerce system built with Rust.

## Quick Start

```bash
docker-compose up -d
curl http://localhost:8083/health
```

## Configuration

| Variable | Description | Default |
|----------|-------------|---------|
| `DATABASE_URL` | PostgreSQL URL | Required |
| `PORT` | Service port | `8083` |
| `NATS_URL` | NATS server | Optional |

## API Endpoints

- `GET /health` - Health check
- `GET /api/v1/*` - API endpoints

## Development

```bash
cp .env.example .env
cargo run
```

## License

MIT OR Apache-2.0

## Billyronks Sovereign Standardization

- Vertical: **Core Platform**
- Benchmark targets: **Atlassian, ServiceNow**
- Event backbone: **Apache Pulsar** (`eventing/pulsar/topics.yaml`)
- Observability/search: **Quickwit** (`observability/quickwit/index-config.yaml`)
- Harvester HCI baseline: **Mayastor/Vitastor-compatible** storage contracts (`infrastructure/kubernetes/harvester/storage-baseline.yaml`)
- Autonomous expansion target: **Cross-domain policy orchestration with event-sourced audit graph**
