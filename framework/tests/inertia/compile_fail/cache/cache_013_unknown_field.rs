use suprnova::InertiaProps;

#[derive(InertiaProps)]
#[inertia(cacheable, public = "missing")]
struct PageProps {
    #[inertia(public)]
    title: String,
}

fn main() {}
