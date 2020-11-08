#[inline(never)]
fn foo(x: usize) -> usize { x + 1 }

fn main() {
    assert_eq!(foo(1), 2);
}