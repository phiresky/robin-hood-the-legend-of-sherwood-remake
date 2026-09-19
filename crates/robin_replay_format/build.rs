mod shared {
    include!("../../build-support/robin_build.rs");
}

fn main() {
    shared::main();
}
