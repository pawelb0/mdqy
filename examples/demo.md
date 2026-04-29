---
title: "Distributed Cache RFC"
status: draft
tags: [storage, consensus]
---

# Distributed Cache

Cache layer in front of S3 with read-through and write-back. Hot tier
on local NVMe, cold tier in object storage.

![architecture](http://example.com/diagram.svg)

## Goals

- p99 read latency under 5 ms on a hit
- 99.9% durability once writes are acknowledged
- Online rebalance under node failure

## Design

### Storage

Each node holds a sorted run of keys assigned via [rendezvous
hashing](http://en.wikipedia.org/wiki/Rendezvous_hashing). Runs are
persisted to NVMe in a log-structured layout.

```rust
pub struct Cache {
    capacity: usize,
    runs: Vec<Run>,
}

impl Cache {
    pub fn get(&self, key: &Key) -> Option<&Value> {
        self.runs.iter().rev().find_map(|r| r.find(key))
    }
}
```

### Consensus

Membership lives in a Raft cluster of 5 nodes. Reads serve from any
node. Writes serialize at the leader.

```python
def quorum_write(leader, value):
    idx = leader.append(value)
    leader.replicate(idx)
    return leader.commit(idx)
```

### Rebalance

Triggered on add/remove. A background scan moves shards to converge
with the new ring.

```sh
ctl rebalance --cluster prod --concurrency 8
```

## API

| Method | Path             | Notes        |
|--------|------------------|--------------|
| GET    | /v1/k/{key}      | read-through |
| PUT    | /v1/k/{key}      | write-back   |
| DELETE | /v1/k/{key}      | tombstoned   |

## Risks

> Consensus traffic dominates cluster bandwidth under heavy churn.
> Mitigate by batching log entries every 5 ms.

## Observability

Metrics exported via [Prometheus](http://prom.example.com).
Dashboards on [Grafana](http://grafana.example.com/d/cache).
