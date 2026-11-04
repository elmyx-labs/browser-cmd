//! The upstream proxy: Chrome goes through it with credentials, never around
//! it, only for the named hosts, and a failed launch leaves no proxy task.

use super::*;

#[tokio::test]
#[ignore = "requires installed Chrome; run with --include-ignored"]
async fn chrome_uses_the_authenticated_proxy_and_does_not_fall_back_directly() {
    use tokio::io::AsyncWriteExt;
    let origin = serve("<!doctype html><h1>Proxy proof</h1><button>Ready</button>".into());
    for allow in [true, false] {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let upstream_address = listener.local_addr().unwrap();
        let attempts = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let authenticated = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(true));
        let (count, auth) = (attempts.clone(), authenticated.clone());
        let proxy = tokio::spawn(async move {
            let mut connections = tokio::task::JoinSet::new();
            loop {
                tokio::select! {
                    accepted = listener.accept() => {
                        let Ok((mut stream, _)) = accepted else { break };
                        let (count, auth) = (count.clone(), auth.clone());
                        connections.spawn(async move {
                            let request = crate::http::read(&mut stream, 8192, 8192).await.unwrap();
                            if request.header("proxy-authorization") != Some("Basic dXNlcjpwYXNz") {
                                auth.store(false, std::sync::atomic::Ordering::SeqCst);
                            }
                            let target = request.start.split_whitespace().nth(1).unwrap_or_default().parse::<std::net::SocketAddr>().ok();
                            if target != Some(origin) || !allow {
                                if target == Some(origin) { count.fetch_add(1, std::sync::atomic::Ordering::SeqCst); }
                                let _ = stream.write_all(b"HTTP/1.1 502 Bad Gateway\r\nContent-Length: 0\r\n\r\n").await;
                                return;
                            }
                            count.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                            let mut target = tokio::net::TcpStream::connect(origin).await.unwrap();
                            stream.write_all(b"HTTP/1.1 200 Connection Established\r\n\r\n").await.unwrap();
                            let _ = tokio::io::copy_bidirectional(&mut stream, &mut target).await;
                        });
                    }
                    _ = connections.join_next(), if !connections.is_empty() => {},
                }
            }
        });
        let temp = tempfile::tempdir().unwrap();
        let mut settings = settings(temp.path(), temp.path());
        settings.attach = None;
        settings.proxy = Some(
            crate::settings::upstream::UpstreamProxy::parse(&format!("http://user:pass@{upstream_address}"))
                .unwrap(),
        );
        settings.chrome_args.push("--disable-background-networking".into());
        let host = Host::new(settings);
        tokio::task::LocalSet::new()
            .run_until(async {
                let mut session = host.session();
                let opened = session.execute("navigate", json!({"url":format!("http://{origin}")})).await;
                if allow {
                    assert!(opened.ok && opened.output.contains("Proxy proof"), "{}", opened.output);
                    let owner = host.shared().await.unwrap();
                    let proxy_id = owner.process.as_ref().unwrap().proxy.as_ref().unwrap().0.id();
                    let old_cdp = owner.cdp.clone();
                    old_cdp.disconnect();
                    drop(owner);
                    let before = attempts.load(std::sync::atomic::Ordering::SeqCst);
                    let again = session
                        .execute("navigate", json!({"url":format!("http://{origin}/after-reconnect")}))
                        .await;
                    assert!(again.ok && again.output.contains("Proxy proof"), "{}", again.output);
                    let recovered = host.shared().await.unwrap();
                    let proxy_task = &recovered.process.as_ref().unwrap().proxy.as_ref().unwrap().0;
                    assert_eq!(proxy_task.id(), proxy_id, "reconnect replaced Chrome's proxy listener");
                    assert!(!proxy_task.is_finished());
                    assert!(
                        attempts.load(std::sync::atomic::Ordering::SeqCst) > before,
                        "reconnected browser did not use its proxy"
                    );
                } else {
                    assert!(!opened.ok, "proxy refusal unexpectedly reached the origin: {}", opened.output);
                }
                assert!(attempts.load(std::sync::atomic::Ordering::SeqCst) > 0, "Chrome bypassed the proxy");
                assert!(authenticated.load(std::sync::atomic::Ordering::SeqCst));
                host.shutdown().await;
            })
            .await;
        proxy.abort();
        let _ = proxy.await;
    }
}

#[test]
fn only_the_named_hosts_go_through_the_proxy_and_they_never_fall_back_direct() {
    let pac = crate::chrome::proxy::pac(&["example.com".into()], "127.0.0.1:9".parse().unwrap());
    let script = percent_encoding::percent_decode_str(
        pac.strip_prefix("data:application/x-ns-proxy-autoconfig,").unwrap(),
    )
    .decode_utf8()
    .unwrap();
    assert_eq!(
        script,
        r#"function FindProxyForURL(u,h){return h=="example.com"||dnsDomainIs(h,".example.com")?"PROXY 127.0.0.1:9":"DIRECT"}"#
    );
}

#[tokio::test]
async fn failed_browser_startup_does_not_leave_a_proxy_task_running() {
    let temp = tempfile::tempdir().unwrap();
    let mut settings = settings(temp.path(), temp.path());
    settings.executable = Some(temp.path().join("missing-chrome"));
    settings.attach = None;
    settings.proxy = Some(crate::settings::upstream::UpstreamProxy::parse("http://127.0.0.1:1").unwrap());
    let metrics = tokio::runtime::Handle::current().metrics();
    let before = metrics.num_alive_tasks();
    let host = Host::new(settings);
    assert!(host.shared().await.is_err());
    tokio::time::timeout(std::time::Duration::from_secs(1), async {
        while metrics.num_alive_tasks() > before {
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("failed launch leaked a proxy task");
}
