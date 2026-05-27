use color_eyre::eyre::Report;
use config::get_config;
use img::Screenshot;
use serde::{de::Error, Deserialize, Deserializer, Serialize, Serializer};
use sss_capture_ui::SelectorMode;
use sss_lib::generate_image;
use tracing_subscriber::EnvFilter;

mod config;
mod error;
mod img;
mod interactive;
mod shot;

#[derive(Clone, Copy, Debug, Default)]
pub struct Area {
    x: i32,
    y: i32,
    width: u32,
    height: u32,
}

fn main() -> Result<(), Report> {
    // Default: warn-and-above for our code, but silence winit-wayland's
    // expected layer-shell complaints (xdg_toplevel / min-max size unsupported
    // — those are inherent to the protocol we deliberately use).
    let env_filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| {
        EnvFilter::new("warn,winit_wayland=error,sctk=error,wayland_client=error")
    });
    let subscriber = tracing_subscriber::FmtSubscriber::builder()
        .with_env_filter(env_filter)
        .with_writer(std::io::stderr)
        .with_timer(tracing_subscriber::fmt::time::Uptime::default())
        .finish();

    tracing::subscriber::set_global_default(subscriber).expect("setting default subscriber failed");

    // install color eyre
    color_eyre::config::HookBuilder::default()
        .issue_url(concat!(env!("CARGO_PKG_REPOSITORY"), "/issues/new"))
        .add_issue_metadata("version", env!("CARGO_PKG_VERSION"))
        .issue_filter(|kind| match kind {
            color_eyre::ErrorKind::NonRecoverable(_) => false,
            color_eyre::ErrorKind::Recoverable(_) => true,
        })
        .install()?;

    let (config, mut g_config, ui_config) = get_config()?;
    if config.verbose {
        // Re-init at info level by overriding the existing filter. We do
        // that lazily here so the verbose flag is read after parsing.
        std::env::set_var("RUST_LOG", "info");
    }
    tracing::trace!("Configs loaded");

    // The CLI is interactive whenever the user did NOT supply a complete
    // targeting flag, OR when `--interactive` is forced.
    //   * `--area "x,y WxH"`              → direct
    //   * `--area`                        → selector in Area mode
    //   * `--screen-id <v>`               → direct
    //   * `--screen-id`                   → selector in Monitor mode
    //   * `--window <v>`                  → direct
    //   * `--window`                      → selector in Window mode
    //   * `--screen --current`            → direct (monitor under cursor)
    //   * `--screen` alone                → selector in Monitor mode
    //   * `--current` alone               → direct (monitor under cursor)
    //   * (none of the above)             → selector in AnyOf mode
    let direct = config.direct_target();
    let want_interactive = config.interactive || direct.is_none();

    if want_interactive {
        let mode = pick_initial_mode(&config);
        // `interactive::run` returns `Ok(None)` for user cancellation
        // (Esc / Cancel button). That's not a real error — it's just
        // "user changed their mind". We exit 1 directly so scripts can
        // detect the cancel, but without color_eyre's big error chrome
        // which would otherwise present cancellation as a crash.
        let pre = match interactive::run(&config, &g_config, &ui_config, mode)? {
            Some(pre) => pre,
            None => std::process::exit(1),
        };
        // The GUI may have flipped Copy / Save intent. Honour them only when
        // the CLI itself didn't already specify them.
        if pre.action.copy && !g_config.copy {
            g_config.copy = true;
        }
        if pre.action.save && (g_config.output.trim().is_empty() || g_config.output == "out.png") {
            if let Some(path) = pre
                .action
                .save_path_hint
                .clone()
                .or_else(|| pre.default_output.clone())
            {
                g_config.output = path.to_string_lossy().into_owned();
            }
        }
        return Ok(generate_image(
            g_config,
            Screenshot::pre_rendered(pre.image),
        )?);
    }

    Ok(generate_image(
        g_config,
        Screenshot::from_target(direct.unwrap(), config.show_cursor),
    )?)
}

/// Decide which mode the selector should open in based on which targeting
/// flag the user supplied without a value.
fn pick_initial_mode(config: &config::CliConfig) -> SelectorMode {
    use config::{AreaSpec, ScreenSpec, WindowSpec};
    if matches!(config.area, Some(AreaSpec::Interactive)) {
        SelectorMode::Area
    } else if matches!(config.window, Some(WindowSpec::Interactive)) {
        SelectorMode::Window
    } else if matches!(config.screen_id, Some(ScreenSpec::Interactive))
        || (config.screen && !config.current)
    {
        SelectorMode::Monitor
    } else {
        SelectorMode::AnyOf
    }
}

fn str_to_area(s: &str) -> Result<Area, String> {
    let err = "The format of area is wrong (x,y WxH)".to_string();
    let (pos, size) = s.split_once(' ').ok_or(err.clone())?;
    let (x, y) = pos.split_once(',').ok_or(err.clone()).map(|(x, y)| {
        (
            x.parse::<i32>().map_err(|e| e.to_string()),
            y.parse::<i32>().map_err(|e| e.to_string()),
        )
    })?;
    let (w, h) = size.split_once('x').ok_or(err.clone()).map(|(w, h)| {
        (
            w.parse::<u32>().map_err(|e| e.to_string()),
            h.parse::<u32>().map_err(|e| e.to_string()),
        )
    })?;

    Ok(Area {
        x: x?,
        y: y?,
        width: w?,
        height: h?,
    })
}

impl<'de> Deserialize<'de> for Area {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        str_to_area(&String::deserialize(deserializer)?).map_err(D::Error::custom)
    }
}

impl Serialize for Area {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let Area {
            x,
            y,
            width,
            height,
        } = self;
        String::serialize(&format!("{x},{y} {width}x{height}"), serializer)
    }
}
