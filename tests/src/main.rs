use std::thread;

fn main() {
    println!("Hello World");
    thread::spawn(move || println!("Hello World")).join();
}