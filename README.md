# prometheus-remote-write

This repository contains a Rust library and a CLI for sending metrics to
Prometheus over the remote write API.

The CLI is very useful for sending a small amount of ad-hoc metrics to Prometheus.

A prime use case is cronjobs.

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

## CLI: Installation

### Release Binaries

You can download release binaries for Windows, Mac OS and Linux from the Github
releases: https://github.com/theduke/prom-write/releases

```bash
# Make sure to replace the <RELEASE_TAG> with the latest release!
curl -L https://github.com/theduke/prom-write/releases/download/<RELEASE_TAG>/prom-write-linux-x86 > prom-write
chmod +x prom-write
./prom-write --help
```

### Nix(OS)

The prom-write binary is not in nixpkgs yet, but you can use the Nix Flake
directly if you have Flake support enabled:

```bash
# Run once
nix run github.com/theduke/prom-write -- --help

# Open a shell
nix shell github.com/theduke/prom-write
prom-write --help
```

### Cargo

If you have Rust and cargo installed, you can install the CLI through the crate:

```bash
cargo install --force prom-write
```

