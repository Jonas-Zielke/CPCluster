use tokio::{net::TcpStream, io::AsyncReadExt, time::{sleep, Duration}};
use std::error::Error;

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error + Send + Sync>> {
    let primary_addr = "127.0.0.1:55000";
    loop {
        match TcpStream::connect(primary_addr).await {
            Ok(mut stream) => {
                println!("Standby master connected to primary");
                let mut buf = [0u8; 1];
                loop {
                    match stream.read(&mut buf).await {
                        Ok(0) | Err(_) => break,
                        Ok(_) => {}
                    }
                }
                println!("Lost connection to primary");
            }
            Err(_) => {
                println!("Unable to connect to primary, acting as master (not implemented)");
            }
        }
        sleep(Duration::from_secs(5)).await;
    }
}
