//! Native file dialogs, run away from the egui update thread.
//!
//! The dialog commands intentionally receive the requested path through an
//! environment variable.  In particular, user supplied Windows paths must not
//! become part of a PowerShell command string.

use std::{
    path::PathBuf,
    process::Command,
    sync::mpsc::{self, Receiver},
    thread,
};

#[cfg(target_os = "macos")]
use std::path::Path;

/// The dialogs used by the desktop application.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Kind {
    Config,
    Directory,
    Results,
}

impl Kind {
    #[cfg(any(target_os = "windows", target_os = "macos"))]
    fn environment_value(self) -> &'static str {
        match self {
            Self::Config => "config",
            Self::Directory => "directory",
            Self::Results => "results",
        }
    }
}

/// Opens a native chooser without stalling the GUI.
///
/// `Ok(None)` means that the user cancelled the dialog. `Err` means that the
/// platform dialog program could not be started or reported a real failure.
pub fn open(kind: Kind, initial: &str) -> Receiver<Result<Option<PathBuf>, String>> {
    let (sender, receiver) = mpsc::channel();
    let initial = initial.to_owned();
    thread::spawn(move || {
        let _ = sender.send(open_blocking(kind, &initial));
    });
    receiver
}

fn open_blocking(kind: Kind, initial: &str) -> Result<Option<PathBuf>, String> {
    #[cfg(target_os = "windows")]
    {
        windows_dialog(kind, initial)
    }
    #[cfg(target_os = "macos")]
    {
        macos_dialog(kind, initial)
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        linux_dialog(kind, initial)
    }
    #[cfg(not(any(target_os = "windows", target_os = "macos", unix)))]
    {
        let _ = (kind, initial);
        Err("native file dialogs are not supported on this platform".to_owned())
    }
}

fn completed_path(
    output: std::process::Output,
    cancel_status: Option<i32>,
) -> Result<Option<PathBuf>, String> {
    if cancel_status.is_some_and(|status| output.status.code() == Some(status)) {
        return Ok(None);
    }
    if !output.status.success() {
        let error = String::from_utf8_lossy(&output.stderr).trim().to_owned();
        return Err(if error.is_empty() {
            format!("file dialog exited with {}", output.status)
        } else {
            format!("file dialog failed: {error}")
        });
    }
    let selected = String::from_utf8_lossy(&output.stdout).trim().to_owned();
    if selected.is_empty() {
        Ok(None)
    } else {
        Ok(Some(PathBuf::from(selected)))
    }
}

#[cfg(any(target_os = "windows", test))]
const WINDOWS_DIALOG_SCRIPT: &str = r#"
Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'
$utf8 = New-Object System.Text.UTF8Encoding($false)
[Console]::OutputEncoding = $utf8
$OutputEncoding = $utf8
Add-Type -AssemblyName System.Windows.Forms
$kind = $env:TRKR_DIALOG_KIND
$initial = $env:TRKR_DIALOG_INITIAL
if ($kind -eq 'directory' -or $kind -eq 'results') {
  $dialog = New-Object System.Windows.Forms.FolderBrowserDialog
  if ($initial) { $dialog.SelectedPath = $initial }
} elseif ($kind -eq 'save_csv') {
  $dialog = New-Object System.Windows.Forms.SaveFileDialog
  $dialog.Filter = 'CSV files (*.csv)|*.csv|All files (*.*)|*.*'
  $dialog.DefaultExt = 'csv'
  $dialog.AddExtension = $true
  if ($initial) { $dialog.FileName = $initial }
} elseif ($kind -eq 'replay_csv') {
  $dialog = New-Object System.Windows.Forms.OpenFileDialog
  $dialog.Filter = 'CSV files (*.csv)|*.csv|All files (*.*)|*.*'
  $dialog.CheckFileExists = $true
  if ($initial) {
    if (Test-Path -LiteralPath $initial -PathType Container) { $dialog.InitialDirectory = $initial }
    else {
      $dialog.FileName = $initial
      $parent = Split-Path -Parent $initial
      if ($parent) { $dialog.InitialDirectory = $parent }
    }
  }
} else {
  $dialog = New-Object System.Windows.Forms.OpenFileDialog
  $dialog.Filter = 'JSON profile (*.json)|*.json|All files (*.*)|*.*'
  $dialog.CheckFileExists = $true
  if ($initial) {
    if (Test-Path -LiteralPath $initial -PathType Container) { $dialog.InitialDirectory = $initial }
    else {
      $dialog.FileName = $initial
      $parent = Split-Path -Parent $initial
      if ($parent) { $dialog.InitialDirectory = $parent }
    }
  }
}
if ($dialog.ShowDialog() -eq [System.Windows.Forms.DialogResult]::OK) {
  if ($kind -eq 'directory') { [Console]::Out.Write($dialog.SelectedPath) }
  else { [Console]::Out.Write($dialog.FileName) }
}
"#;

#[cfg(target_os = "windows")]
fn windows_dialog(kind: Kind, initial: &str) -> Result<Option<PathBuf>, String> {
    use std::os::windows::process::CommandExt;

    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    let mut command = Command::new("powershell");
    command
        .args([
            "-NoProfile",
            "-NonInteractive",
            "-STA",
            "-Command",
            WINDOWS_DIALOG_SCRIPT,
        ])
        .env("TRKR_DIALOG_KIND", kind.environment_value())
        .env("TRKR_DIALOG_INITIAL", initial)
        .creation_flags(CREATE_NO_WINDOW);
    let output = command
        .output()
        .map_err(|error| format!("could not start PowerShell file dialog: {error}"))?;
    completed_path(output, None)
}

#[cfg(target_os = "macos")]
const MACOS_DIALOG_SCRIPT: &str = r#"
set dialogKind to system attribute "TRKR_DIALOG_KIND"
set initialDir to system attribute "TRKR_DIALOG_INITIAL_DIR"
set initialName to system attribute "TRKR_DIALOG_INITIAL_NAME"
try
  if dialogKind is "directory" or dialogKind is "results" then
    if initialDir is not "" then
      set picked to choose folder with prompt "Choose output folder" default location (POSIX file initialDir)
    else
      set picked to choose folder with prompt "Choose output folder"
    end if
  else if dialogKind is "save_csv" then
    if initialDir is not "" then
      set picked to choose file name with prompt "Save CSV" default name initialName default location (POSIX file initialDir)
    else
      set picked to choose file name with prompt "Save CSV" default name initialName
    end if
  else if dialogKind is "replay_csv" then
    if initialDir is not "" then
      set picked to choose file with prompt "Choose replay CSV" default location (POSIX file initialDir) of type {"public.comma-separated-values-text"}
    else
      set picked to choose file with prompt "Choose replay CSV" of type {"public.comma-separated-values-text"}
    end if
  else
    if initialDir is not "" then
      set picked to choose file with prompt "Choose JSON profile" default location (POSIX file initialDir) of type {"public.json"}
    else
      set picked to choose file with prompt "Choose JSON profile" of type {"public.json"}
    end if
  end if
  return POSIX path of picked
on error number -128
  return ""
end try
"#;

#[cfg(target_os = "macos")]
fn macos_dialog(kind: Kind, initial: &str) -> Result<Option<PathBuf>, String> {
    let (initial_dir, initial_name) = macos_initial_location(initial);
    let output = Command::new("osascript")
        .args(["-e", MACOS_DIALOG_SCRIPT])
        .env("TRKR_DIALOG_KIND", kind.environment_value())
        .env("TRKR_DIALOG_INITIAL_DIR", initial_dir)
        .env("TRKR_DIALOG_INITIAL_NAME", initial_name)
        .output()
        .map_err(|error| format!("could not start macOS file dialog: {error}"))?;
    completed_path(output, None)
}

#[cfg(target_os = "macos")]
fn macos_initial_location(initial: &str) -> (String, String) {
    let path = Path::new(initial);
    let directory = if path.is_dir() {
        Some(path)
    } else {
        path.parent()
            .filter(|parent| !parent.as_os_str().is_empty())
    }
    .filter(|parent| parent.is_dir())
    .map(|parent| parent.to_string_lossy().into_owned())
    .unwrap_or_default();
    let filename = path
        .file_name()
        .filter(|name| !name.is_empty())
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| "output.csv".to_owned());
    (directory, filename)
}

#[cfg(all(unix, not(target_os = "macos")))]
fn linux_dialog(kind: Kind, initial: &str) -> Result<Option<PathBuf>, String> {
    let mut command = Command::new("zenity");
    command.arg("--file-selection");
    match kind {
        Kind::Config => {
            command.arg("--file-filter=JSON profile | *.json");
        }
        Kind::Directory | Kind::Results => {
            command.arg("--directory");
        }
    }
    if !initial.is_empty() {
        command.arg(format!("--filename={initial}"));
    }
    let output = command
        .output()
        .map_err(|error| format!("could not start zenity file dialog: {error}"))?;
    completed_path(output, Some(1))
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    #[cfg(unix)]
    fn output(status: i32, stdout: &[u8]) -> std::process::Output {
        use std::os::unix::process::ExitStatusExt;

        std::process::Output {
            status: std::process::ExitStatus::from_raw(status << 8),
            stdout: stdout.to_vec(),
            stderr: Vec::new(),
        }
    }

    #[test]
    fn windows_script_uses_environment_for_user_paths() {
        assert!(super::WINDOWS_DIALOG_SCRIPT.contains("$env:TRKR_DIALOG_INITIAL"));
        assert!(!super::WINDOWS_DIALOG_SCRIPT.contains("{initial}"));
    }

    #[test]
    fn windows_script_returns_the_selected_file_name() {
        assert!(super::WINDOWS_DIALOG_SCRIPT.contains("$dialog.FileName"));
        assert!(super::WINDOWS_DIALOG_SCRIPT.contains("$dialog.SelectedPath"));
        assert!(super::WINDOWS_DIALOG_SCRIPT.contains("UTF8Encoding"));
        assert!(super::WINDOWS_DIALOG_SCRIPT.contains("Set-StrictMode"));
    }

    #[test]
    fn replay_csv_is_an_open_dialog_with_a_csv_filter() {
        assert!(super::WINDOWS_DIALOG_SCRIPT.contains("replay_csv"));
        assert!(super::WINDOWS_DIALOG_SCRIPT.contains("CSV files (*.csv)"));
    }

    #[test]
    #[cfg(unix)]
    fn completed_path_preserves_a_successful_selection_and_linux_cancel() {
        assert_eq!(
            super::completed_path(output(0, b"/tmp/measure.csv\n"), Some(1)).unwrap(),
            Some(PathBuf::from("/tmp/measure.csv"))
        );
        assert_eq!(
            super::completed_path(output(1, b""), Some(1)).unwrap(),
            None
        );
    }
}
