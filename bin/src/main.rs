//! prom-write: CLI for writing metrics to Prometheus over the remote-write API.

use std::{collections::HashMap, io::Read, time::Duration};

use anyhow::{bail, Context};
use prometheus_remote_write::{Label, TimeSeries, WriteRequest, LABEL_NAME};

fn main() -> Result<(), anyhow::Error> {
    let cli_args = std::env::args().skip(1).collect::<Vec<_>>();
    let mut stdout = std::io::stdout().lock();
    let mut stderr = std::io::stderr().lock();
    run(cli_args, &mut stdout, &mut stderr)
}

fn run(
    args: Vec<String>,
    stdout: &mut impl std::io::Write,
    stderr: &mut impl std::io::Write,
) -> Result<(), anyhow::Error> {
    let cmd = Cmd::parse(&args)?;
    cmd.run(stdout, stderr)
}

const fn crate_version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}

#[derive(Clone, Debug, PartialEq)]
struct Args {
    url: url::Url,
    timeout: Option<Duration>,
    input: MetricOrFile,
    headers: http::HeaderMap,
}

#[derive(Clone, Debug, PartialEq)]
enum Cmd {
    Help,
    Version,
    Run(Args),
}

impl Cmd {
    fn run(
        self,
        stdout: &mut impl std::io::Write,
        stderr: &mut impl std::io::Write,
    ) -> Result<(), anyhow::Error> {
        match self {
            Cmd::Help => {
                writeln!(stdout, "{}", Self::usage())?;
                Ok(())
            }
            Cmd::Version => {
                writeln!(stdout, "prom-write {}", crate_version())?;
                Ok(())
            }
            Cmd::Run(args) => {
                let user_agent = format!("prom-write/{}", crate_version());

                // Sort labels by name, and the samples by timestamp, according to the spec.
                let req = args.build_http_req(&user_agent)?;

                let (parts, body) = req.into_parts();

                let timeout = args.timeout.unwrap_or_else(|| Duration::from_secs(60));
                let agent = ureq::builder().timeout(timeout).build();

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

                writeln!(stderr, "Metrics written successfully")?;
                Ok(())
            }
        }
    }

    fn parse(args: &[String]) -> Result<Cmd, anyhow::Error> {
        let mut url: Option<url::Url> = None;

        // single metric
        let mut help = false;
        let mut version = false;
        let mut name: Option<String> = None;
        let mut kind: Option<MetricType> = None;
        let mut labels = HashMap::<String, String>::new();
        let mut number: Option<f64> = None;
        let mut headers = http::HeaderMap::new();
        let mut timeout: Option<Duration> = None;

        // input file
        let mut input_file: Option<String> = None;

        let mut index = 0;
        while index < args.len() {
            let value = &args[index];

            match value.as_str() {
                "--help" => {
                    help = true;
                    break;
                }
                "--version" => {
                    version = true;
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
                "-h" | "--header" => {
                    index += 1;
                    let (key, val) = args
                        .get(index)
                        .context("-h/--header argument requires a value (header pair X=Y)")?
                        .trim()
                        .split_once('=')
                        .context("-h/--header argument requires a key-value pair (X=Y)")?;

                    let name = key.trim();
                    if name.is_empty() {
                        bail!("argument -h/--header requires a non-empty key: '{key}={val}'");
                    }

                    let key: http::HeaderName = name
                        .parse()
                        .context("argument -h/--header: invalid header name '{name}'")?;
                    let value: http::HeaderValue = val
                        .parse()
                        .context("argument -h/--header: invalid header value: '{val}'")?;

                    headers.insert(key, value);
                    index += 1;
                }
                "--timeout" => {
                    index += 1;
                    let value = args
                        .get(index)
                        .context("--timeout argument requires a value (timeout in seconds)")?
                        .trim()
                        .parse::<u64>()
                        .context("--timeout argument requires a number (timeout in seconds)")?;
                    timeout = Some(Duration::from_secs(value));
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
            Ok(Cmd::Help)
        } else if version {
            Ok(Cmd::Version)
        } else {
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
            Ok(Cmd::Run(Args {
                url,
                headers,
                timeout,
                input,
            }))
        }
    }

    fn usage() -> String {
        const USAGE: &str = r#"prom-write ${version}

Write metrics to Prometheus over the remote-write API

Arguments:
  -h, --help
    Print this help message and exit.

  -u, --url <url>: required!
    Prometheus remote write endpoint URL

  -h, --header KEY=VALUE
    Specify additional custom headers to send in the http request.

  --timeout <timeout:SECONDS>
    Timeout for the HTTP request. If not specified, the default is 60 seconds.

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

        USAGE.replace("${version}", crate_version())
    }

    #[cfg(test)]
    fn try_into_run(self) -> Result<Args, Self> {
        if let Self::Run(v) = self {
            Ok(v)
        } else {
            Err(self)
        }
    }
}

impl Args {
    fn build_write_request(&self) -> Result<WriteRequest, anyhow::Error> {
        match &self.input {
            MetricOrFile::Metric {
                name,
                kind: _,
                labels,
                value,
            } => {
                let mut labels = labels
                    .iter()
                    .map(|(k, v)| Label {
                        name: k.clone(),
                        value: v.clone(),
                    })
                    .collect::<Vec<_>>();
                labels.push(Label {
                    name: LABEL_NAME.to_string(),
                    value: name.clone(),
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
                        value: *value,
                        timestamp: time,
                    }],
                }];

                Ok(WriteRequest { timeseries })
            }
            MetricOrFile::File(path) => {
                let contents = if path == "-" {
                    let mut stdin = std::io::stdin().lock();
                    let mut buf = String::new();
                    stdin.read_to_string(&mut buf)?;
                    buf
                } else {
                    std::fs::read_to_string(path)
                        .with_context(|| format!("could not read file '{path}'"))?
                };

                prometheus_remote_write::WriteRequest::from_text_format(contents).map_err(|err| {
                    anyhow::anyhow!("could not parse input as Prometheus text format: {err}")
                })
            }
        }
    }

    fn build_http_req(&self, user_agent: &str) -> Result<http::Request<Vec<u8>>, anyhow::Error> {
        let req = self.build_write_request()?;
        let mut h = req
            .build_http_request(&self.url, user_agent)
            .map_err(|err| anyhow::anyhow!("could not build HTTP request: {err}"))?;

        for name in self.headers.keys() {
            for value in self.headers.get_all(name) {
                h.headers_mut().insert(name, value.clone());
            }
        }

        Ok(h)
    }
}

#[derive(Clone, Debug, PartialEq)]
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

#[cfg(test)]
mod tests {
    use std::{io::Write, str::FromStr};

    use http::HeaderMap;
    use prometheus_remote_write::Sample;

    use super::*;

    fn mkargs(args: impl IntoIterator<Item = impl Into<String>>) -> Vec<String> {
        args.into_iter().map(Into::into).collect()
    }

    fn mkheaders(
        args: impl IntoIterator<Item = (impl Into<String>, impl Into<String>)>,
    ) -> http::HeaderMap {
        let mut headers = http::HeaderMap::new();
        for (k, v) in args {
            let k = http::HeaderName::from_str(&k.into()).unwrap();
            let v = http::HeaderValue::from_str(&v.into()).unwrap();
            headers.insert(k, v);
        }
        headers
    }

    fn req_reset_timestamp(req: &mut WriteRequest) {
        for series in &mut req.timeseries {
            for sample in &mut series.samples {
                sample.timestamp = 0;
            }
        }
    }

    #[test]
    fn test_parse_help() {
        let cmd = Cmd::parse(&mkargs(["--help"])).unwrap();
        assert_eq!(cmd, Cmd::Help);
    }

    #[test]
    fn test_parse_version() {
        let cmd = Cmd::parse(&mkargs(["--version"])).unwrap();
        assert_eq!(cmd, Cmd::Version);
    }

    #[test]
    fn test_parse_args_file_sparse_short() {
        let cmd = Cmd::parse(&mkargs(["-u", "http://test.com", "-f", "test.txt"])).unwrap();
        assert_eq!(
            cmd,
            Cmd::Run(Args {
                url: "http://test.com".parse().unwrap(),
                timeout: None,
                input: MetricOrFile::File("test.txt".to_string()),
                headers: HeaderMap::new(),
            })
        );
    }

    #[test]
    fn test_parse_url_arg_only_once() {
        let err = Cmd::parse(&mkargs([
            "-u",
            "http://test.com",
            "-f",
            "test.txt",
            "-u",
            "http://test2.com",
        ]))
        .unwrap_err();
        assert!(err.to_string().contains("only be specified once"));
    }

    #[test]
    fn test_parse_url_without_value() {
        let err = Cmd::parse(&mkargs(["-u"])).unwrap_err();
        assert!(err.to_string().contains("requires a value"));
    }

    #[test]
    fn test_parse_header_arg_without_value() {
        let err = Cmd::parse(&mkargs(["-h"])).unwrap_err();
        assert!(err.to_string().contains("requires a value"));
    }

    #[test]
    fn test_parse_header_without_eq() {
        let err = Cmd::parse(&mkargs(["-h", "a"])).unwrap_err();
        assert!(err.to_string().contains("key-value pair"));
    }

    #[test]
    fn test_parse_header_name_invalid() {
        let err = Cmd::parse(&mkargs(["-h", "ööáá=123"])).unwrap_err();
        assert!(err.to_string().contains("invalid header name"));
    }

    #[test]
    fn test_parse_invalid_header_value() {
        let val = "öö\0\t\náá";
        assert!(http::HeaderValue::from_str(val).is_err());

        let err = Cmd::parse(&mkargs(["-h", &format!("a={val}")])).unwrap_err();
        assert!(err.to_string().contains("invalid header value"));
    }

    #[test]
    fn test_parse_file_once() {
        let err = Cmd::parse(&mkargs(["-f", "test.txt", "--file", "test2.txt"])).unwrap_err();
        assert!(err.to_string().contains("only be specified once"));
    }

    #[test]
    fn test_parse_name_once() {
        let err = Cmd::parse(&mkargs(["-n", "name", "--name", "name2"])).unwrap_err();
        assert!(err.to_string().contains("only be specified once"));
    }

    #[test]
    fn test_parse_name_without_value() {
        let err = Cmd::parse(&mkargs(["-n"])).unwrap_err();
        assert!(err.to_string().contains("requires a value"));
    }

    #[test]
    fn test_parse_name_with_empty_value() {
        let err = Cmd::parse(&mkargs(["-n", ""])).unwrap_err();
        assert!(err.to_string().contains("requires a non-empty value"));
    }

    #[test]
    fn test_parse_type_once() {
        let err = Cmd::parse(&mkargs(["-t", "counter", "--type", "gauge"])).unwrap_err();
        assert!(err.to_string().contains("only be specified once"));
    }

    #[test]
    fn test_parse_type_without_value() {
        let err = Cmd::parse(&mkargs(["-t"])).unwrap_err();
        assert!(err.to_string().contains("requires a value"));
    }

    #[test]
    fn test_parse_url_invalid() {
        let err = Cmd::parse(&mkargs(["--url", "/////!"])).unwrap_err();
        assert!(err.to_string().contains("invalid url"));
    }

    #[test]
    fn test_parse_empty_header_key() {
        let err = Cmd::parse(&mkargs(["-h", "=lala"])).unwrap_err();
        assert!(err.to_string().contains("empty key"));
    }

    #[test]
    fn test_parse_type_summary() {
        let err = Cmd::parse(&mkargs(["-t", "summary"])).unwrap_err();
        assert!(err.to_string().contains("not supported yet"));
    }

    #[test]
    fn test_parse_type_histogram() {
        let err = Cmd::parse(&mkargs(["-t", "histogram"])).unwrap_err();
        assert!(err.to_string().contains("not supported yet"));
    }

    #[test]
    fn test_parse_unknown_type() {
        let err = Cmd::parse(&mkargs(["-t", "unknown"])).unwrap_err();
        assert!(err.to_string().contains("unknown metric type"));
    }

    #[test]
    fn test_parse_value_once() {
        let err = Cmd::parse(&mkargs(["-v", "1.5", "--value", "2.5"])).unwrap_err();
        assert!(err.to_string().contains("only be specified once"));
    }

    #[test]
    fn test_parse_value_without_value() {
        let err = Cmd::parse(&mkargs(["-v"])).unwrap_err();
        assert!(err.to_string().contains("requires a value"));
    }

    #[test]
    fn test_parse_value_not_a_number() {
        let err = Cmd::parse(&mkargs(["-v", "a"])).unwrap_err();
        assert!(err.to_string().contains("requires a number"));
    }

    #[test]
    fn test_parse_label_without_value() {
        let err = Cmd::parse(&mkargs(["-l"])).unwrap_err();
        assert!(err.to_string().contains("requires a value"));
    }

    #[test]
    fn test_parse_label_no_key_value() {
        let err = Cmd::parse(&mkargs(["-l", "a"])).unwrap_err();
        assert!(err.to_string().contains("key-value pair"));
    }

    #[test]
    fn test_parse_label_empty_key() {
        let err = Cmd::parse(&mkargs(["-l", "=a"])).unwrap_err();
        assert!(err.to_string().contains("empty key"));
    }

    #[test]
    fn test_parse_label_empty_value() {
        let err = Cmd::parse(&mkargs(["-l", "a="])).unwrap_err();
        assert!(err.to_string().contains("empty value"));
    }

    #[test]
    fn test_parse_unknown_argument() {
        let err = Cmd::parse(&mkargs(["--unknown"])).unwrap_err();
        assert!(err.to_string().contains("unknown argument"));
    }

    #[test]
    fn test_parse_missing_url() {
        let err = Cmd::parse(&mkargs(["-f", "test.txt"])).unwrap_err();
        assert!(err.to_string().contains("missing required argument -u"));
    }

    #[test]
    fn test_parse_metric_with_missing_args() {
        let err = Cmd::parse(&mkargs(["-u", "http://local", "-n", "name"])).unwrap_err();
        assert!(err.to_string().contains("missing required argument -v"));

        let err = Cmd::parse(&mkargs(["-u", "http://local", "-v", "1.5"])).unwrap_err();
        assert!(err
            .to_string()
            .contains("missing required argument -n/--name"));
    }

    #[test]
    fn test_parse_autodetect_counter_type() {
        let args = Cmd::parse(&mkargs([
            "-u",
            "http://local",
            "-n",
            "x_total",
            "-v",
            "123",
        ]))
        .unwrap()
        .try_into_run()
        .unwrap();

        assert_eq!(
            args.input,
            MetricOrFile::Metric {
                name: "x_total".to_string(),
                kind: MetricType::Counter,
                labels: HashMap::new(),
                value: 123.0,
            }
        );
    }

    #[test]
    fn test_parse_file_with_metric_args() {
        let err = Cmd::parse(&mkargs([
            "-u",
            "http://local",
            "-f",
            "test.txt",
            "-n",
            "name",
        ]))
        .unwrap_err();
        assert!(err.to_string().contains("cannot be used with"));

        let err = Cmd::parse(&mkargs([
            "-u",
            "http://local",
            "--file",
            "test.txt",
            "-t",
            "counter",
        ]))
        .unwrap_err();
        assert!(err.to_string().contains("cannot be used with"));

        let err = Cmd::parse(&mkargs([
            "-u",
            "http://local",
            "-f",
            "test.txt",
            "-v",
            "1.5",
        ]))
        .unwrap_err();
        assert!(err.to_string().contains("cannot be used with"));

        let err = Cmd::parse(&mkargs([
            "-u",
            "http://local",
            "-f",
            "test.txt",
            "-l",
            "a=b",
        ]))
        .unwrap_err();
        assert!(err.to_string().contains("cannot be used with"));
    }

    #[test]
    fn test_parse_args_file_full_short() {
        let cmd = Cmd::parse(&mkargs([
            "-u",
            "http://test.com",
            "-f",
            "test.txt",
            "-h",
            "a=a123",
            "--timeout",
            "11",
            "--header",
            "blub=lala5",
        ]))
        .unwrap();
        assert_eq!(
            cmd,
            Cmd::Run(Args {
                url: "http://test.com".parse().unwrap(),
                timeout: Some(Duration::from_secs(11)),
                input: MetricOrFile::File("test.txt".to_string()),
                headers: mkheaders([
                    ("a".to_string(), "a123".to_string()),
                    ("blub".to_string(), "lala5".to_string())
                ]),
            })
        );
    }

    #[test]
    fn test_parse_file_full_long() {
        let cmd = Cmd::parse(&mkargs([
            "--url",
            "http://test.com:8080",
            "--file",
            "test.txt",
            "-h",
            "a=a123",
            "--timeout",
            "11",
            "--header",
            "blub=lala5",
        ]))
        .unwrap();
        assert_eq!(
            cmd,
            Cmd::Run(Args {
                url: "http://test.com:8080".parse().unwrap(),
                timeout: Some(Duration::from_secs(11)),
                input: MetricOrFile::File("test.txt".to_string()),
                headers: mkheaders([
                    ("a".to_string(), "a123".to_string()),
                    ("blub".to_string(), "lala5".to_string())
                ]),
            })
        );
    }

    #[test]
    fn test_parse_args_metric_sparse_short() {
        let cmd = Cmd::parse(&mkargs([
            "-u",
            "http://test.com",
            "-n",
            "name",
            "-v",
            "1.5",
        ]))
        .unwrap();
        assert_eq!(
            cmd,
            Cmd::Run(Args {
                url: "http://test.com".parse().unwrap(),
                timeout: None,
                input: MetricOrFile::Metric {
                    name: "name".to_string(),
                    kind: MetricType::Gauge,
                    labels: HashMap::new(),
                    value: 1.5,
                },
                headers: HeaderMap::new(),
            })
        );

        let mut write_req = cmd.try_into_run().unwrap().build_write_request().unwrap();
        req_reset_timestamp(&mut write_req);

        assert_eq!(
            write_req,
            WriteRequest {
                timeseries: vec![TimeSeries {
                    labels: vec![Label {
                        name: "__name__".to_string(),
                        value: "name".to_string(),
                    }],
                    samples: vec![Sample {
                        value: 1.5,
                        timestamp: 0,
                    },]
                }]
            }
        );
    }

    #[test]
    fn test_parse_args_metric_full_short() {
        let cmd = Cmd::parse(&mkargs([
            "-u",
            "http://test.com",
            "-t",
            "gauge",
            "-n",
            "name",
            "-v",
            "1.5",
            "-l",
            "alph123=valval123",
            "-l",
            "l2=v2",
            "--label",
            "l3=vv3",
            "-h",
            "h1=a123",
        ]))
        .unwrap();
        assert_eq!(
            cmd,
            Cmd::Run(Args {
                url: "http://test.com".parse().unwrap(),
                timeout: None,
                input: MetricOrFile::Metric {
                    name: "name".to_string(),
                    kind: MetricType::Gauge,
                    labels: vec![
                        ("alph123".to_string(), "valval123".to_string()),
                        ("l2".to_string(), "v2".to_string()),
                        ("l3".to_string(), "vv3".to_string())
                    ]
                    .into_iter()
                    .collect(),
                    value: 1.5,
                },
                headers: mkheaders([("h1".to_string(), "a123".to_string())]),
            })
        );

        let hreq = cmd.try_into_run().unwrap().build_http_req("test").unwrap();
        let (parts, _body) = hreq.into_parts();
        assert_eq!(parts.headers.get("h1").unwrap(), "a123");
    }

    #[test]
    fn test_parse_args_metric_full_long() {
        let args = Cmd::parse(&mkargs([
            "--url",
            "http://test.com",
            "--name",
            "name",
            "--value",
            "1.5",
            "--type",
            "counter",
            "--label",
            "alph123=valval123",
            "-l",
            "l2=v2",
            "--label",
            "l3=vv3",
            "--header",
            "h1=a123",
            "--timeout",
            "123",
        ]))
        .unwrap()
        .try_into_run()
        .unwrap();
        assert_eq!(
            args,
            Args {
                url: "http://test.com".parse().unwrap(),
                timeout: Some(Duration::from_secs(123)),
                input: MetricOrFile::Metric {
                    name: "name".to_string(),
                    kind: MetricType::Counter,
                    labels: vec![
                        ("alph123".to_string(), "valval123".to_string()),
                        ("l2".to_string(), "v2".to_string()),
                        ("l3".to_string(), "vv3".to_string())
                    ]
                    .into_iter()
                    .collect(),
                    value: 1.5,
                },
                headers: mkheaders([("h1".to_string(), "a123".to_string())]),
            }
        );
    }

    fn run_capture(args: Vec<String>) -> Result<(String, String), anyhow::Error> {
        let mut stdout = Vec::<u8>::new();
        let mut stderr = Vec::<u8>::new();
        run(args, &mut stdout, &mut stderr)?;
        Ok((String::from_utf8(stdout)?, String::from_utf8(stderr)?))
    }

    #[test]
    fn test_run_version() {
        let (stdout, _stderr) = run_capture(mkargs(["--version"])).unwrap();
        assert!(stdout.contains("prom-write"));
        assert!(stdout.contains(crate_version()));
    }

    #[test]
    fn test_run_help() {
        let (stdout, _stderr) = run_capture(mkargs(["--help"])).unwrap();
        assert!(stdout.contains("prom-write"));
        assert!(stdout.contains("--url"));
    }

    // Make sure writing to a vec works as expected.
    #[test]
    fn test_vec_write() {
        let mut buf = Vec::<u8>::new();

        writeln!(&mut buf, "test").unwrap();
        writeln!(&mut buf, "test2").unwrap();

        assert_eq!(buf, b"test\ntest2\n");
    }
}
