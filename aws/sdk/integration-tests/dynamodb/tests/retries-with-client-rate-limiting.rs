/*
 * Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
 * SPDX-License-Identifier: Apache-2.0
 */

#[cfg(aws_sdk_orchestrator_mode)]
mod test {
    use aws_sdk_dynamodb::config::{Credentials, Region, SharedAsyncSleep};
    use aws_sdk_dynamodb::{config::retry::RetryConfig, error::ProvideErrorMetadata};
    use aws_smithy_async::rt::sleep::TokioSleep;
    use aws_smithy_async::test_util::instant_time_and_sleep;
    use aws_smithy_async::time::SharedTimeSource;
    use aws_smithy_async::time::SystemTimeSource;
    use aws_smithy_client::test_connection::TestConnection;
    use aws_smithy_http::body::SdkBody;
    use aws_smithy_runtime::client::retries::RetryPartition;
    use aws_smithy_runtime_api::client::orchestrator::{HttpRequest, HttpResponse};
    use aws_smithy_types::timeout::TimeoutConfigBuilder;
    use std::time::{Duration, Instant, SystemTime};

    fn req() -> HttpRequest {
        http::Request::builder()
            .body(SdkBody::from("request body"))
            .unwrap()
    }

    fn ok() -> HttpResponse {
        http::Response::builder()
            .status(200)
            .header("server", "Server")
            .header("content-type", "application/x-amz-json-1.0")
            .header("content-length", "23")
            .header("connection", "keep-alive")
            .header("x-amz-crc32", "2335643545")
            .body(SdkBody::from("{ \"TableNames\": [ \"Test\" ] }"))
            .unwrap()
    }

    fn err() -> HttpResponse {
        http::Response::builder()
            .status(500)
            .body(SdkBody::from("{ \"message\": \"The request has failed because of an unknown error, exception or failure.\", \"code\": \"InternalServerError\" }"))
            .unwrap()
    }

    fn throttling_err() -> HttpResponse {
        http::Response::builder()
            .status(400)
            .body(SdkBody::from("{ \"message\": \"The request was denied due to request throttling.\", \"code\": \"ThrottlingException\" }"))
            .unwrap()
    }

    #[tokio::test]
    async fn test_adaptive_retries_with_no_throttling_errors() {
        let (time_source, sleep_impl) = instant_time_and_sleep(SystemTime::UNIX_EPOCH);

        let events = vec![
            // First operation
            (req(), err()),
            (req(), err()),
            (req(), ok()),
            // Second operation
            (req(), err()),
            (req(), ok()),
            // Third operation will fail, only errors
            (req(), err()),
            (req(), err()),
            (req(), err()),
            (req(), err()),
        ];

        let conn = TestConnection::new(events);
        let config = aws_sdk_dynamodb::Config::builder()
            .credentials_provider(Credentials::for_tests())
            .region(Region::new("us-east-1"))
            .retry_config(
                RetryConfig::adaptive()
                    .with_max_attempts(4)
                    .with_use_static_exponential_base(true),
            )
            .time_source(SharedTimeSource::new(time_source))
            .sleep_impl(SharedAsyncSleep::new(sleep_impl.clone()))
            .retry_partition(RetryPartition::new(
                "test_adaptive_retries_with_no_throttling_errors",
            ))
            .http_connector(conn.clone())
            .build();
        let expected_table_names = vec!["Test".to_owned()];

        // We create a new client each time to ensure that the cross-client retry state is working.
        let client = aws_sdk_dynamodb::Client::from_conf(config.clone());
        let res = client.list_tables().send().await.unwrap();
        assert_eq!(sleep_impl.total_duration(), Duration::from_secs(3));
        assert_eq!(res.table_names(), Some(expected_table_names.as_slice()));
        // Three requests should have been made, two failing & one success
        assert_eq!(conn.requests().len(), 3);

        let client = aws_sdk_dynamodb::Client::from_conf(config.clone());
        let res = client.list_tables().send().await.unwrap();
        assert_eq!(sleep_impl.total_duration(), Duration::from_secs(3 + 1));
        assert_eq!(res.table_names(), Some(expected_table_names.as_slice()));
        // Two requests should have been made, one failing & one success (plus previous requests)
        assert_eq!(conn.requests().len(), 5);

        let client = aws_sdk_dynamodb::Client::from_conf(config);
        let err = client.list_tables().send().await.unwrap_err();
        assert_eq!(sleep_impl.total_duration(), Duration::from_secs(3 + 1 + 7),);
        assert_eq!(err.code(), Some("InternalServerError"));
        // four requests should have been made, all failing (plus previous requests)
        assert_eq!(conn.requests().len(), 9);
    }

    #[tokio::test]
    async fn test_adaptive_retries_with_throttling_errors_times_out() {
        tracing_subscriber::fmt::init();
        let events = vec![
            // First operation
            (req(), err()),
            (req(), ok()),
            // Second operation
            (req(), err()),
            (req(), throttling_err()),
            (req(), ok()),
        ];

        let conn = TestConnection::new(events);
        let config = aws_sdk_dynamodb::Config::builder()
            .credentials_provider(Credentials::for_tests())
            .region(Region::new("us-east-1"))
            .retry_config(
                RetryConfig::adaptive()
                    .with_max_attempts(4)
                    .with_initial_backoff(Duration::from_millis(50))
                    .with_use_static_exponential_base(true),
            )
            .timeout_config(
                TimeoutConfigBuilder::new()
                    .operation_attempt_timeout(Duration::from_millis(100))
                    .build(),
            )
            .time_source(SharedTimeSource::new(SystemTimeSource::new()))
            .sleep_impl(SharedAsyncSleep::new(TokioSleep::new()))
            .http_connector(conn.clone())
            .retry_partition(RetryPartition::new(
                "test_adaptive_retries_with_throttling_errors_times_out",
            ))
            .build();

        let expected_table_names = vec!["Test".to_owned()];
        let start = Instant::now();

        // We create a new client each time to ensure that the cross-client retry state is working.
        let client = aws_sdk_dynamodb::Client::from_conf(config.clone());
        let res = client.list_tables().send().await.unwrap();
        assert_eq!(res.table_names(), Some(expected_table_names.as_slice()));
        // Three requests should have been made, two failing & one success
        assert_eq!(conn.requests().len(), 2);

        let client = aws_sdk_dynamodb::Client::from_conf(config);
        let err = client.list_tables().send().await.unwrap_err();
        assert_eq!(err.to_string(), "request has timed out".to_owned());
        // two requests should have been made, both failing (plus previous requests)
        assert_eq!(conn.requests().len(), 2 + 2);

        let since = start.elapsed();
        // At least 300 milliseconds must pass:
        // - 50ms for the first retry on attempt 1
        // - 50ms for the second retry on attempt 3
        // - 100ms for the throttling delay triggered by attempt 4, which required a delay longer than the attempt timeout.
        // - 100ms for the 5th attempt, which would have succeeded, but required a delay longer than the attempt timeout.
        assert!(since.as_secs_f64() > 0.3);
    }
}