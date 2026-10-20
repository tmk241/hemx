#[tokio::main(flavor = "current_thread")]
async fn main() -> std::io::Result<()> {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:3000").await?;
    println!("Hemx counter: http://127.0.0.1:3000 (Ctrl-C to stop)");
    axum::serve(listener, hemx_counter_example::app()).await
}
