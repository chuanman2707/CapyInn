use std::fs as std_fs;
use std::time::Instant;
use tokio::fs as tokio_fs;
use tokio::task;

#[tokio::main]
async fn main() {
    let iterations = 500;
    let data =
        "Some random data to write to a file, maybe quite a bit so it takes some time ".repeat(100);

    // Create a dir
    std_fs::create_dir_all("bench_data").unwrap();

    // Baseline: std::fs
    println!("Running std::fs benchmark...");
    let start_std = Instant::now();
    let mut handles_std = vec![];
    for i in 0..iterations {
        let path = format!("bench_data/file_std_{}.txt", i);
        let data_clone = data.clone();
        handles_std.push(task::spawn(async move {
            std_fs::write(&path, data_clone).unwrap();
        }));
    }
    for handle in handles_std {
        handle.await.unwrap();
    }
    let duration_std = start_std.elapsed();
    println!("std::fs duration: {:?}", duration_std);

    // Optimized: tokio::fs
    println!("Running tokio::fs benchmark...");
    let start_tokio = Instant::now();
    let mut handles = vec![];
    for i in 0..iterations {
        let path = format!("bench_data/file_tokio_{}.txt", i);
        let data_clone = data.clone();
        handles.push(task::spawn(async move {
            tokio_fs::write(&path, data_clone).await.unwrap();
        }));
    }
    for handle in handles {
        handle.await.unwrap();
    }
    let duration_tokio = start_tokio.elapsed();
    println!("tokio::fs duration: {:?}", duration_tokio);

    // Cleanup
    std_fs::remove_dir_all("bench_data").unwrap();
}
