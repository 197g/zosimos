use zosimos::{
    buffer::ByteLayout,
    command::{CommandBuffer, Rectangle},
};

#[test]
fn simple_program() {
    use zosimos::pool::Pool;

    const BACKGROUND: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/input/background.png");
    const FOREGROUND: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/input/foreground.png");

    let mut pool = Pool::new();
    let mut commands = CommandBuffer::default();

    let background = image::open(BACKGROUND).expect("Background image opened");
    let foreground = image::open(FOREGROUND).expect("Background image opened");
    let expected = ByteLayout::from(&background);

    let placement = Rectangle {
        x: 0,
        y: 0,
        max_x: foreground.width(),
        max_y: foreground.height(),
    };

    let background = pool.insert_srgb(&background);
    let background = commands.input_from(background.into());

    let foreground = pool.insert_srgb(&foreground);
    let foreground = commands.input_from(foreground.into());

    let result = commands
        .inscribe(background, placement, foreground)
        .expect("Valid to inscribe");
    let (_, outformat) = commands.output(result).expect("Valid for output");

    let linker = zosimos_std::from_included();

    let _ = linker
        .compile(&commands)
        .expect("Could build command buffer");
    assert_eq!(outformat.as_concrete().map(|x| x.layout), Some(expected));
}
