use std::time::Duration;

#[derive(Default)]
pub(crate) struct ActivationLock(pub(crate) tokio::sync::Mutex<()>);

pub(crate) struct ApiClient(pub(crate) reqwest::Client);

impl Default for ApiClient {
    fn default() -> Self {
        Self(
            crate::network::client_builder()
                .expect("无法读取系统代理设置")
                .redirect(reqwest::redirect::Policy::none())
                .connect_timeout(Duration::from_secs(30))
                .timeout(Duration::from_secs(60))
                .pool_max_idle_per_host(4)
                .pool_idle_timeout(Duration::from_secs(90))
                .tcp_keepalive(Duration::from_secs(60))
                .build()
                .expect("无法初始化 HTTP 客户端"),
        )
    }
}
