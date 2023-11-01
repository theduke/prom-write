# Changelog - prom-write


## [0.2.1] - 2023-11-01

### Features

- Extend counter type autodetection

### Bug Fixes

- Fix text format parsing + time series sorting

### Documentation

- Fix Nix instructions

### Testing

- Add lots of tests

## [0.2.0] - 2023-11-01

### Features

- Add request --timeout argument:
    allows configuring http request timeout in seconds
- Add -h, --header argument for speciyfing custom headers
    Note: only --help works for help now, -h is for headers
- Add --version flag + show version in help

## [0.1.0] - 2023-11-01

Initial release with a working Rust library and CLI (prom-write).

