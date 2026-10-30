// SPDX-FileCopyrightText: 2026 Foundation Devices, Inc. <hello@foundation.xyz>
// SPDX-License-Identifier: MIT

// App theming.
//
// The app theme is authored as JSON (the source of truth) at the path named by
// `theme` in app-config.toml and compiled to Rust by the Foundation CLI. Use
// `foundation theme` to edit it visually.
//
// Semantic colors are pinned below as runtime overrides as well: SDK 1.1.0
// normalizes some JSON colors (including button foregrounds) to base defaults.

slint_keyos_platform::settings::use_api!(
    slint_keyos_platform::settings,
    slint_keyos_platform::server
);

use foundation_themes::ColorScheme;
use slint_keyos_platform::slint::ComponentHandle;

foundation_themes::include_theme!(app_theme);

pub fn init(ui: &crate::AppWindow) {
    apply_system_theme(ui, SettingsApi::default().get_system_theme());

    let ui_weak = ui.as_weak();
    let mut updates = slint_keyos_platform::subscribe_scalar::<
        settings_permissions::SettingsPermissions,
        _,
    >(settings_permissions::settings::messages::SubscribeSystemTheme);
    slint_keyos_platform::spawn_local(async move {
        while let Some(system_theme) = updates.next().await {
            let Some(ui) = ui_weak.upgrade() else {
                break;
            };
            apply_system_theme(&ui, system_theme);
        }
    })
    .detach();
}

fn apply_system_theme(
    ui: &crate::AppWindow,
    system_theme: settings_permissions::settings::global::SystemTheme,
) {
    let scheme = match system_theme {
        settings_permissions::settings::global::SystemTheme::Dark => ColorScheme::Dark,
        settings_permissions::settings::global::SystemTheme::Light => ColorScheme::Light,
    };

    // Apply the app theme to every component binding (palette, button styles,
    // sizes, named text-role font sizes).
    foundation_themes::apply_theme!(ui, app_theme::theme(), scheme);

    // Pin semantic colors after the SDK normalizes inherited tokens.
    // Keep these in sync with theme.json; primary buttons need a dark foreground.
    let theme = ui.global::<crate::Theme>();
    let rgb = slint_keyos_platform::slint::Color::from_rgb_u8;
    theme.set_palette_primary(rgb(214, 154, 120));
    theme.set_palette_primary_pressed(rgb(188, 128, 96));
    theme.set_palette_background(rgb(16, 17, 19));
    theme.set_palette_surface(rgb(28, 30, 32));
    theme.set_palette_card(rgb(28, 30, 32));
    theme.set_palette_foreground(rgb(247, 245, 240));
    theme.set_palette_muted(rgb(164, 167, 172));
    theme.set_palette_secondary(rgb(48, 51, 54));
    theme.set_palette_border(rgb(65, 69, 74));
    theme.set_palette_danger(rgb(239, 114, 114));
    theme.set_color_primary(rgb(214, 154, 120));
    theme.set_color_primary_dark(rgb(188, 128, 96));
    theme.set_color_primary_light(rgb(222, 174, 147));
    theme.set_color_secondary(rgb(48, 51, 54));
    theme.set_color_secondary_dark(rgb(40, 43, 46));
    theme.set_color_foreground(rgb(247, 245, 240));
    theme.set_color_muted(rgb(164, 167, 172));
    theme.set_color_background(rgb(16, 17, 19));
    theme.set_color_surface(rgb(28, 30, 32));
    theme.set_color_border(rgb(65, 69, 74));
    theme.set_color_white(rgb(16, 17, 19));
    theme.set_color_success(rgb(67, 217, 163));
    theme.set_color_warning(rgb(243, 186, 98));
    theme.set_color_danger(rgb(239, 114, 114));
}
