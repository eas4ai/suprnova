use suprnova::InertiaProps;

#[derive(InertiaProps)]
#[inertia(cacheable)]
struct PageProps {
    #[inertia(public)]
    title: String,
    viewer: String,
}

fn main() {}
