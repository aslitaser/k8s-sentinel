# k8s-sentinel

Kubernetes admission webhook that validates and mutates workloads against a configurable set of policies.

## Policies

- **resource_limits** — reject containers exceeding CPU/memory caps, optionally inject default requests/limits
- **image_registry** — restrict images to an allowlist of registries, block `:latest`
- **labels** — require specific metadata labels (with optional regex validation)
- **topology_spread** — enforce topology spread constraints, optionally inject them

Each policy can run in `enforce` (reject) or `warn` (allow + warning header) mode.

## Architecture

Two servers:
- HTTPS on `:8443` — `/validate` and `/mutate` webhook endpoints (2 MiB body limit)
- HTTP on `:9090` — `/healthz`, `/readyz`, `/metrics` (Prometheus/OpenMetrics)

## Running locally

```
./generate-certs.sh
cargo run -- --config config/policies.yaml
```

## Configuration

See [`config/policies.yaml`](config/policies.yaml) for all options. Every setting can be overridden via environment variables with `SENTINEL_` prefix (nested with `__`, e.g. `SENTINEL_POLICIES__ENFORCE_RESOURCE_LIMITS__ENABLED=true`).

## Deploying

Kustomize bases and overlays are in `deploy/k8s/`. ArgoCD manifests in `deploy/argocd/`.

```
# dev (kind cluster)
kubectl apply -k deploy/k8s/overlays/dev

# prod
kubectl apply -k deploy/k8s/overlays/prod
```
