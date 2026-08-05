use std::time::Duration;

#[derive(Default)]
pub(crate) struct ActivationLock(pub(crate) tokio::sync::Mutex<()>);

#[derive(Default)]
pub(crate) struct ApiClient(pub(crate) crate::network::ClientCache);

impl ApiClient {
    pub(crate) fn current(&self) -> Result<reqwest::Client, crate::AppError> {
        self.0
            .current(|builder| {
                builder
                    .redirect(reqwest::redirect::Policy::none())
                    .connect_timeout(Duration::from_secs(30))
                    .timeout(Duration::from_secs(60))
                    .pool_max_idle_per_host(4)
                    .pool_idle_timeout(Duration::from_secs(90))
                    .tcp_keepalive(Duration::from_secs(60))
                    .build()
            })
            .map_err(|error| {
                crate::AppError::Internal(format!("无法初始化网络客户端：{}", error.without_url()))
            })
    }

    pub(crate) fn invalidate(&self) {
        self.0.invalidate();
    }
}
