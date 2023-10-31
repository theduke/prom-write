# prometheus-remote-write

This repository contains a Rust library and a CLI for sending metrics to
Prometheus over the remote write API.

The CLI is very useful for sending a small amount of ad-hoc metrics to Prometheus.

A prime use case is cronjobs.

## CLI: Installation

TODO

## CLI: Usage

```bash
# The help output contains detailed explanations and examples.
prom-write --help

# Consult the help for more examples, the below is a sneak peek.

prom-write --url http://localhost:9090/api/v1/write --name requests --value 1
prom-write --url http://localhost:9090/api/v1/write -n requests -t counter -v 1
prom-write --url http://localhost:9090/api/v1/write -n requests -v 1 --label method=GET -l path=/api/v1/write
prom-write --url http://localhost:9090/api/v1/write --file metrics.txt -l instance=localhost
```


