//! prom-write: CLI for writing metrics to Prometheus over the remote-write API.

use std::{collections::HashMap, io::Read};

use anyhow::{bail, Context};
use prometheus_remote_write::{Label, TimeSeries, WriteRequest, LABEL_NAME};

fn main() {
    run().unwrap();
}

fn run() -> Result<(), anyhow::Error> {
    let cli_args = std::env::args().skip(1).collect::<Vec<_>>();
    let args = Args::parse(&cli_args)?;

    let req = match args.input {
        MetricOrFile::Metric {
            name,
            kind: _,
            labels,
            value,
        } => {
            let mut labels = labels
                .into_iter()
                .map(|(k, v)| Label { name: k, value: v })
                .collect::<Vec<_>>();
            labels.push(Label {
                name: LABEL_NAME.to_string(),
                value: name,
            });

            let time: i64 = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_millis()
                .try_into()
                .expect("timestamp is too large");

            let timeseries = vec![TimeSeries {
                labels,
                samples: vec![prometheus_remote_write::Sample {
                    value,
                    timestamp: time,
                }],
            }];

            WriteRequest { timeseries }
        }
        MetricOrFile::File(path) => {
            let contents = if path == "-" {
                let mut stdin = std::io::stdin().lock();
                let mut buf = String::new();
                stdin.read_to_string(&mut buf)?;
                buf
            } else {
                std::fs::read_to_string(&path)
                    .with_context(|| format!("could not read file '{path}'"))?
            };

            prometheus_remote_write::WriteRequest::from_text_format(contents).map_err(|err| {
                anyhow::anyhow!("could not parse input as Prometheus text format: {err}")
            })?
        }
    };

    // Content-Encoding: snappy
    // Content-Type: application/x-protobuf
    // User-Agent: <name & version of the sender>
    // X-Prometheus-Remote-Write-Version: 0.1.0

    let user_agent = format!("prom-write/{}", env!("CARGO_PKG_VERSION"));

    // Sort labels by name, and the samples by timestamp, according to the spec.
    let req = req
        .build_http_request(&args.url, &user_agent)
        .map_err(|err| anyhow::anyhow!("could not build HTTP request: {err}"))?;

    let (parts, body) = req.into_parts();

    let agent = ureq::builder().build();

    let mut req = agent.request(parts.method.as_str(), &parts.uri.to_string());
    for key in parts.headers.keys() {
        for value in parts.headers.get_all(key) {
            req = req.set(
                key.as_str(),
                value.to_str().context("non-utf8 http header value")?,
            );
        }
    }

    let res = req
        .send_bytes(&body)
        .context("could not send HTTP request")?;
    let status = res.status();
    if !(200..=299).contains(&status) {
        bail!("server returned error status code {status}");
    }

    Ok(())
}

#[derive(Clone, Debug)]
struct Args {
    url: url::Url,
    input: MetricOrFile,
}

#[derive(Clone, Debug)]
enum MetricOrFile {
    Metric {
        name: String,
        #[allow(dead_code)]
        kind: MetricType,
        labels: HashMap<String, String>,
        value: f64,
    },
    File(String),
}

#[allow(dead_code)]
#[derive(Clone, Debug, PartialEq, Eq)]
enum MetricType {
    Counter,
    Gauge,
    Summary,
    Histogram,
    Untyped,
}

impl Args {
    const USAGE: &'static str = r#"

prom-write - write metrics to Prometheus over the remote-write API

Arguments:
  -h, --help
    Print this help message and exit.

  -u, --url <url>: required!
    Prometheus remote write endpoint URL

Read metrics from file:
  -f, --file <path>:
    Read metrics from a file encoded in the Prometheus text format.
    If the path is '-', read from stdin.

Manually specify metric:
  -n, --name <name:string>: required!
    Metric name

  -v, --value <value:float>: required!
    Metric value

  -t, --type <type:[counter,gauge]>:
    Metric type. Supported types: counter, gauge.
    DEFAULT: counter if name ends with '_total', gauge otherwise.

  -l, --label <key>=<value>:
    Add a label to the metric. Can be specified multiple times.
      

Examples:

* Write a gauge:
  > prom-write --url http://localhost:9090/api/v1/write --name requests --value 1

* Write a counter:
  > prom-write --url http://localhost:9090/api/v1/write -n requests_total -v 1

* Specify the type:
  > prom-write --url http://localhost:9090/api/v1/write -n requests -t counter -v 1

* Add labels:
  > prom-write --url http://localhost:9090/api/v1/write -n requests -v 1 --label method=GET -l path=/api/v1/write

* Write metrics from a file:
  > prom-write --url http://localhost:9090/api/v1/write --file metrics.txt -l instance=localhost

* Write metrics from stdin
  > prom-write --url http://localhost:9090/api/v1/write -f -

"#;

    fn parse(args: &[String]) -> Result<Args, anyhow::Error> {
        let mut url: Option<url::Url> = None;

        // single metric
        let mut name: Option<String> = None;
        let mut kind: Option<MetricType> = None;
        let mut labels = HashMap::<String, String>::new();
        let mut number: Option<f64> = None;
        let mut help = false;

        // input file
        let mut input_file: Option<String> = None;

        let mut index = 0;
        while index < args.len() {
            let value = &args[index];

            match value.as_str() {
                "-h" | "--help" => {
                    help = true;
                    break;
                }
                "-u" | "--url" => {
                    if url.is_some() {
                        bail!("argument -u/--url can only be specified once");
                    }
                    index += 1;

                    let value = args
                        .get(index)
                        .context("-u/--url argument requires a value (Prometheus URL)")?;

                    let value = url::Url::parse(value)
                        .with_context(|| "invalid url '{value}' for argument -u/--url")?;
                    url = Some(value);
                    index += 1;
                }
                "-f" | "--file" => {
                    if input_file.is_some() {
                        bail!("argument -f/--file can only be specified once");
                    }
                    index += 1;

                    let value = args
                        .get(index)
                        .context("-i/--input argument requires a value (file path)")?;

                    input_file = Some(value.clone());
                    index += 1;
                }
                "-n" | "--name" => {
                    if name.is_some() {
                        bail!("argument -n/--name can only be specified once");
                    }
                    index += 1;
                    let value = args
                        .get(index)
                        .context("-n/--name argument requires a value (metric name)")?
                        .trim()
                        .to_string();
                    if value.is_empty() {
                        bail!("argument -n/--name requires a non-empty value");
                    }
                    name = Some(value.clone());
                    index += 1;
                }
                "-t" | "--type" => {
                    if kind.is_some() {
                        bail!("argument -t/--type can only be specified once");
                    }
                    index += 1;
                    let value = args
                        .get(index)
                        .context("-t/--type argument requires a value (metric type)")?
                        .trim()
                        .to_string();
                    let k = match value.as_str() {
                        "counter" => MetricType::Counter,
                        "gauge" => MetricType::Gauge,
                        "histogram" | "summary" => {
                            bail!("metric type '{value}' is not supported yet")
                        }
                        // "untyped" => prometheus::proto::MetricType::UNTYPED,
                        other => bail!("unknown metric type '{other}'"),
                    };
                    kind = Some(k);
                    index += 1;
                }
                "-v" | "--value" => {
                    if number.is_some() {
                        bail!("argument -v/--value can only be specified once");
                    }
                    index += 1;
                    let v = args
                        .get(index)
                        .context("-v/--value argument requires a value (number)")?
                        .trim()
                        .parse::<f64>()
                        .context("-v/--value argument requires a number")?;
                    number = Some(v);
                    index += 1;
                }
                "-l" | "--label" => {
                    index += 1;
                    let (key, val) = args
                        .get(index)
                        .context("-l/--label argument requires a value (label pair X=Y)")?
                        .trim()
                        .split_once('=')
                        .context("-l/--label argument requires a key-value pair (X=Y)")?;
                    let key = key.trim();
                    let val = val.trim();

                    if key.is_empty() {
                        bail!("argument -l/--label requires a non-empty key: '{key}={val}'");
                    }
                    if val.is_empty() {
                        bail!("argument -l/--label requires a non-empty value: '{key}={val}'");
                    }

                    labels.insert(key.to_string(), val.to_string());
                    index += 1;
                }
                other => {
                    bail!("unknown argument '{other}'");
                }
            }
        }

        if help {
            println!("{}", Self::USAGE);
            std::process::exit(0);
        }

        let url = url.context("missing required argument -u/--url")?;

        let input = if let Some(f) = input_file {
            if name.is_some() {
                bail!("argument -n/--name cannot be used with -f/--file");
            }
            if kind.is_some() {
                bail!("argument -t/--type cannot be used with -f/--file");
            }
            if number.is_some() {
                bail!("argument -v/--value cannot be used with -f/--file");
            }
            if !labels.is_empty() {
                bail!("argument -l/--label cannot be used with -f/--file");
            }

            MetricOrFile::File(f)
        } else {
            let name = name.context("missing required argument -n/--name")?;
            let value = number.context("missing required argument -v/--value")?;
            let kind = match kind {
                Some(k) => k,
                None => {
                    if name.ends_with("_total") {
                        MetricType::Counter
                    } else {
                        MetricType::Gauge
                    }
                }
            };

            MetricOrFile::Metric {
                name,
                kind,
                labels,
                value,
            }
        };

        Ok(Args { url, input })
    }
}
