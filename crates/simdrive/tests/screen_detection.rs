//! Regression scenes for the screen-readiness gate, including the former home/review ambiguity.
//! Regions cover whole controls; samples must stay clear of rounded edges, labels, and the footer.
use fps_simdrive::{shows_result, shows_review};
use image::{DynamicImage, ImageFormat, Rgb, RgbImage};
use std::io::Cursor;

const BACKGROUND: [u8; 3] = [16, 17, 19];
const COPPER: [u8; 3] = [214, 154, 120];
const CHARCOAL: [u8; 3] = [48, 51, 54];

fn paint(image: &mut RgbImage, x: u32, y: u32, width: u32, height: u32, colour: [u8; 3]) {
    for row in y..y + height {
        for col in x..x + width {
            image.put_pixel(col, row, Rgb(colour));
        }
    }
}

fn png(image: RgbImage) -> Vec<u8> {
    let mut bytes = Cursor::new(Vec::new());
    DynamicImage::ImageRgb8(image)
        .write_to(&mut bytes, ImageFormat::Png)
        .unwrap();
    bytes.into_inner()
}

fn review(accent: [u8; 3]) -> RgbImage {
    let mut image = RgbImage::from_pixel(480, 800, Rgb(BACKGROUND));
    paint(&mut image, 24, 632, 132, 72, accent);
    paint(&mut image, 24, 716, 432, 60, CHARCOAL);
    image
}

#[test]
fn review_requires_the_slider_and_both_ends_of_reject() {
    let bytes = png(review(COPPER));
    assert!(shows_review(&bytes));
    assert!(!shows_result(&bytes));
    let mut partial = review(COPPER);
    paint(&mut partial, 240, 716, 216, 60, BACKGROUND);
    assert!(
        !shows_review(&png(partial)),
        "a partially drawn Reject row is not ready"
    );
}

#[test]
fn home_never_matches_review_even_when_old_samples_look_like_buttons() {
    for offset in -3..=3 {
        let mut home = RgbImage::from_pixel(480, 800, Rgb(BACKGROUND));
        paint(&mut home, 24, (600 + offset) as u32, 432, 80, COPPER);
        paint(&mut home, 24, (696 + offset) as u32, 432, 48, CHARCOAL);
        let bytes = png(home);
        assert!(!shows_review(&bytes), "home with {offset}px footer shift");
        assert!(!shows_result(&bytes), "home with {offset}px footer shift");
    }
}

#[test]
fn result_requires_a_complete_copper_bottom_button() {
    let mut result = RgbImage::from_pixel(480, 800, Rgb(BACKGROUND));
    paint(&mut result, 24, 712, 432, 64, COPPER);
    let bytes = png(result.clone());
    assert!(shows_result(&bytes));
    assert!(!shows_review(&bytes));
    paint(&mut result, 240, 712, 216, 64, BACKGROUND);
    assert!(!shows_result(&png(result)));
}

#[test]
fn blue_or_channel_swapped_controls_are_not_accepted_as_copper() {
    let blue = [COPPER[2], COPPER[1], COPPER[0]];
    assert!(!shows_review(&png(review(blue))));
    let mut result = RgbImage::from_pixel(480, 800, Rgb(BACKGROUND));
    paint(&mut result, 24, 712, 432, 64, blue);
    assert!(!shows_result(&png(result)));
}

#[test]
fn malformed_and_undersized_screens_are_not_ready() {
    for bytes in [vec![], vec![0, 1, 2], png(RgbImage::new(16, 16))] {
        assert!(!shows_review(&bytes));
        assert!(!shows_result(&bytes));
    }
}

#[test]
fn light_theme_uses_its_own_copper_and_neutral_controls() {
    let light_copper = [149, 86, 51];
    let light_secondary = [232, 229, 223];
    let mut light_review = review(light_copper);
    paint(&mut light_review, 24, 716, 432, 60, light_secondary);
    assert!(shows_review(&png(light_review)));
    let mut light_result = RgbImage::from_pixel(480, 800, Rgb([247, 245, 240]));
    paint(&mut light_result, 24, 712, 432, 64, light_copper);
    assert!(shows_result(&png(light_result)));
    let swapped = [light_copper[2], light_copper[1], light_copper[0]];
    assert!(!shows_review(&png(review(swapped))));
}

#[test]
fn standard_sdk_screens_distinguish_home_review_and_result() {
    let home = include_bytes!("screens/home.png");
    let review = include_bytes!("screens/review.png");
    let result = include_bytes!("screens/result.png");
    assert!(!shows_review(home));
    assert!(!shows_result(home));
    assert!(shows_review(review));
    assert!(!shows_result(review));
    assert!(!shows_review(result));
    assert!(shows_result(result));
}
