//! Types and utilities for calling Prometheus remote write API endpoints.

/// Special label for the name of a metric.
pub const LABEL_NAME: &str = "__name__";
pub const CONTENT_TYPE: &str = "application/x-protobuf";
pub const HEADER_NAME_REMOTE_WRITE_VERSION: &str = "X-Prometheus-Remote-Write-Version";
pub const REMOTE_WRITE_VERSION_01: &str = "0.1.0";

/// A write request.
///
/// .proto:
/// ```protobuf
/// message WriteRequest {
///   repeated TimeSeries timeseries = 1;
///   // Cortex uses this field to determine the source of the write request.
///   // We reserve it to avoid any compatibility issues.
///   reserved  2;

///   // Prometheus uses this field to send metadata, but this is
///   // omitted from v1 of the spec as it is experimental.
///   reserved  3;
/// }
/// ```
#[derive(prost::Message, Clone, PartialEq)]
pub struct WriteRequest {
    #[prost(message, repeated, tag = "1")]
    pub timeseries: Vec<TimeSeries>,
}

impl WriteRequest {
    /// Prepare the write request for sending.
    ///
    /// Ensures that the request conforms to the specification.
    /// See https://prometheus.io/docs/concepts/remote_write_spec.
    pub fn sort(&mut self) {
        for series in &mut self.timeseries {
            series.sort_labels_and_samples();
        }
    }

    pub fn sorted(mut self) -> Self {
        self.sort();
        self
    }

    /// Encode this write request as a protobuf message.
    ///
    /// NOTE: The API requires snappy compression, not a raw protobuf message.
    pub fn encode_proto3(self) -> Vec<u8> {
        prost::Message::encode_to_vec(&self.sorted())
    }

    /// Encode this write request as a snappy-compressed protobuf message.
    #[cfg(feature = "compression")]
    pub fn encode_compressed(self) -> Result<Vec<u8>, snap::Error> {
        snap::raw::Encoder::new().compress_vec(&self.encode_proto3())
    }

    /// Parse metrics from the Prometheus text format, and convert them into a
    /// [`WriteRequest`].
    #[cfg(feature = "parse")]
    pub fn from_text_format(
        text: String,
    ) -> Result<Self, Box<dyn std::error::Error + Send + Sync>> {
        fn samples_to_timeseries(
            samples: Vec<prometheus_parse::Sample>,
        ) -> Result<Vec<TimeSeries>, Box<dyn std::error::Error + Send + Sync>> {
            let mut all_series = std::collections::HashMap::<String, TimeSeries>::new();

            for sample in &samples {
                let mut labels = sample
                    .labels
                    .iter()
                    .map(|(k, v)| (k.as_str(), v.as_str()))
                    .collect::<Vec<_>>();

                labels.push((LABEL_NAME, sample.metric.as_str()));

                labels.sort_by(|a, b| a.0.cmp(b.0));

                let mut ident = sample.metric.clone();
                ident.push_str("_$$_");
                for (k, v) in &labels {
                    ident.push_str(k);
                    ident.push('=');
                    ident.push_str(v);
                }

                let series = all_series.entry(ident).or_insert_with(|| {
                    let labels = labels
                        .iter()
                        .map(|(k, v)| Label {
                            name: k.to_string(),
                            value: v.to_string(),
                        })
                        .collect::<Vec<_>>();

                    TimeSeries {
                        labels,
                        samples: vec![],
                    }
                });

                let value = match sample.value {
                    prometheus_parse::Value::Counter(v) => v,
                    prometheus_parse::Value::Gauge(v) => v,
                    prometheus_parse::Value::Histogram(_) => {
                        Err("histogram not supported yet".to_string())?
                    }
                    prometheus_parse::Value::Summary(_) => {
                        Err("summary not supported yet".to_string())?
                    }
                    prometheus_parse::Value::Untyped(v) => v,
                };

                series.samples.push(Sample {
                    value,
                    timestamp: sample.timestamp.timestamp_millis(),
                });
            }

            Ok(all_series.into_values().collect())
        }

        let iter = text.trim().lines().map(|x| Ok(x.to_string()));
        let parsed = prometheus_parse::Scrape::parse(iter)
            .map_err(|err| format!("could not parse input as Prometheus text format: {err}"))?;

        let mut series = samples_to_timeseries(parsed.samples)?;
        series.sort_by(|a, b| {
            let name_a = a.labels.iter().find(|x| x.name == LABEL_NAME).unwrap();
            let name_b = b.labels.iter().find(|x| x.name == LABEL_NAME).unwrap();
            name_a.value.cmp(&name_b.value)
        });

        let s = Self { timeseries: series };

        Ok(s.sorted())
    }

    /// Build a fully prepared HTTP request that an be sent to a remote write endpoint.
    #[cfg(feature = "http")]
    pub fn build_http_request(
        self,
        endpoint: &url::Url,
        user_agent: &str,
    ) -> Result<http::Request<Vec<u8>>, Box<dyn std::error::Error + Send + Sync>> {
        let req = http::Request::builder()
            .method(http::Method::POST)
            .uri(endpoint.as_str())
            .header(http::header::CONTENT_TYPE, CONTENT_TYPE)
            .header(HEADER_NAME_REMOTE_WRITE_VERSION, REMOTE_WRITE_VERSION_01)
            .header(http::header::CONTENT_ENCODING, "snappy")
            .header(http::header::USER_AGENT, user_agent)
            .body(self.encode_compressed()?)?;

        Ok(req)
    }
}

/// A time series.
///
/// .proto:
/// ```protobuf
/// message TimeSeries {
///   repeated Label labels   = 1;
///   repeated Sample samples = 2;
/// }
/// ```
#[derive(prost::Message, Clone, PartialEq)]
pub struct TimeSeries {
    #[prost(message, repeated, tag = "1")]
    pub labels: Vec<Label>,
    #[prost(message, repeated, tag = "2")]
    pub samples: Vec<Sample>,
}

impl TimeSeries {
    /// Sort labels by name, and the samples by timestamp.
    ///
    /// Required by the specification.
    pub fn sort_labels_and_samples(&mut self) {
        self.labels.sort_by(|a, b| a.name.cmp(&b.name));
        self.samples.sort_by(|a, b| a.timestamp.cmp(&b.timestamp));
    }
}

/// A label.
///
/// .proto:
/// ```protobuf
/// message Label {
///   string name  = 1;
///   string value = 2;
/// }
/// ```
#[derive(prost::Message, Clone, Hash, PartialEq, Eq)]
pub struct Label {
    #[prost(string, tag = "1")]
    pub name: String,
    #[prost(string, tag = "2")]
    pub value: String,
}

/// A sample.
///
/// .proto:
/// ```protobuf
/// message Sample {
///   double value    = 1;
///   int64 timestamp = 2;
/// }
/// ```
#[derive(prost::Message, Clone, PartialEq)]
pub struct Sample {
    #[prost(double, tag = "1")]
    pub value: f64,
    #[prost(int64, tag = "2")]
    pub timestamp: i64,
}

#[cfg(all(feature = "parse", feature = "compression"))]
#[cfg(test)]
mod tests {
    use pretty_assertions::assert_eq;

    use super::*;
    #[test]
    fn test_name() {
        let input = r#"
# TYPE mycounter counter
# TYPE mygauge gauge

mygauge 100 100
http_requests_total{method="post",code="200"} 1027 1395066363000
mycounter 100 100
alpha 10 1000
http_requests_total{method="post",code="200"} 50 1000
    "#;

        let req = WriteRequest::from_text_format(input.to_string()).unwrap();

        assert_eq!(
            req,
            WriteRequest {
                timeseries: vec![
                    TimeSeries {
                        labels: vec![Label {
                            name: LABEL_NAME.to_string(),
                            value: "alpha".to_string()
                        },],
                        samples: vec![Sample {
                            value: 10.0,
                            timestamp: 1000,
                        },]
                    },
                    TimeSeries {
                        labels: vec![
                            Label {
                                name: LABEL_NAME.to_string(),
                                value: "http_requests_total".to_string()
                            },
                            Label {
                                name: "code".to_string(),
                                value: "200".to_string()
                            },
                            Label {
                                name: "method".to_string(),
                                value: "post".to_string()
                            },
                        ],
                        samples: vec![
                            Sample {
                                value: 50.0,
                                timestamp: 1000,
                            },
                            Sample {
                                value: 1027.0,
                                timestamp: 1395066363000
                            },
                        ]
                    },
                    TimeSeries {
                        labels: vec![Label {
                            name: LABEL_NAME.to_string(),
                            value: "mycounter".to_string()
                        },],
                        samples: vec![Sample {
                            value: 100.0,
                            timestamp: 100,
                        }],
                    },
                    TimeSeries {
                        labels: vec![Label {
                            name: LABEL_NAME.to_string(),
                            value: "mygauge".to_string()
                        },],
                        samples: vec![Sample {
                            value: 100.0,
                            timestamp: 100,
                        }],
                    },
                ]
            }
        );

        let _x = req.clone().encode_proto3();
        let _y = req.encode_compressed();
    }
}
