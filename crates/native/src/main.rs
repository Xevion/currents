#[snafu::report]
fn main() -> Result<(), currents::Error> {
    currents::run()
}
