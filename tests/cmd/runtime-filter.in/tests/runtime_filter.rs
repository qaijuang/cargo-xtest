//@ run-flags: --test-threads 1

#[test]
fn selected_case() {
    println!("selected test ran inside Microsandbox");
}

#[test]
fn unselected_case() {
    panic!("the positional test filter was not forwarded to libtest");
}
