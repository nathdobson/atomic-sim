use std::thread;

fn main() {
    thread::spawn(move || println!("Hello World")).join();
}