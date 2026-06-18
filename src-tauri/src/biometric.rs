//! Native biometric unlock — macOS Touch ID / Windows Hello — used to gate the
//! saved Robinhood session behind a local presence check.
//!
//! This is an *app-enforced* gate, not a keychain access-control list: the OS
//! keychain still protects the OAuth token at rest. Biometrics here decide
//! whether TrendWave will reveal or use that already-stored session in the
//! current run, so a connected account isn't exposed the moment the app opens.

use robius_authentication::{
    AndroidText, BiometricStrength, Context, Error as BioError, Policy, PolicyBuilder, Text,
    WindowsText,
};

use crate::error::{AppError, AppResult};

/// Whether this build targets a platform with a supported device-auth backend.
///
/// We deliberately avoid a hardware probe: the underlying API can only report
/// real availability by attempting a prompt, so we report platform support here
/// and let [`authenticate`] surface the precise reason (nothing enrolled, no
/// passcode, unavailable) if the user actually tries to unlock.
pub fn is_available() -> bool {
    cfg!(any(target_os = "macos", target_os = "windows"))
}

/// Show the native authentication prompt, blocking on a worker thread until the
/// user responds.
///
/// - `Ok(true)`  — the user authenticated; the caller may unlock.
/// - `Ok(false)` — the user dismissed or failed the prompt; stay locked quietly.
/// - `Err(_)`    — biometrics can't be used as configured (e.g. nothing enrolled
///   and no device passcode); the caller should surface the message.
pub async fn authenticate(reason: &str) -> AppResult<bool> {
    let reason = reason.to_string();
    // `blocking_authenticate` parks the calling thread on a channel until the
    // system reply block fires, so keep it off the async runtime's threads.
    tokio::task::spawn_blocking(move || blocking_prompt(&reason))
        .await
        .map_err(|_| AppError::Biometric("the authentication prompt did not complete".into()))?
}

fn blocking_prompt(reason: &str) -> AppResult<bool> {
    // Touch ID / Windows Hello with device-password fallback (`password(true)`),
    // so a machine without biometric hardware can still authenticate and nobody
    // is ever locked out of their own saved session.
    let policy: Policy = PolicyBuilder::new()
        .biometrics(Some(BiometricStrength::Strong))
        .password(true)
        .watch(true)
        .build()
        .ok_or_else(|| {
            AppError::Biometric("biometric authentication is not supported here".into())
        })?;

    let text = Text {
        android: AndroidText {
            title: "Unlock TrendWave",
            subtitle: None,
            description: Some(reason),
        },
        // Apple renders this as "TrendWave is trying to <reason>".
        apple: reason,
        windows: WindowsText::new_truncated("Unlock TrendWave", reason),
    };

    match Context::new(()).blocking_authenticate(text, &policy) {
        Ok(()) => Ok(true),
        Err(err) => classify(err),
    }
}

/// Split native errors into a quiet "stay locked" (`Ok(false)`) for user-driven
/// dismissals and retryable failures, versus a surfaced configuration problem.
fn classify(err: BioError) -> AppResult<bool> {
    match err {
        // The user actively dismissed or mistyped — let them retry without a
        // scary banner.
        BioError::UserCanceled
        | BioError::AppCanceled
        | BioError::SystemCanceled
        | BioError::Authentication
        | BioError::Exhausted
        | BioError::NotInteractive => Ok(false),
        // Everything below means it won't work until the user changes something.
        BioError::NotEnrolled => Err(AppError::Biometric(
            "No biometrics or device password are enrolled. Set up Touch ID (or a login password) \
             to use unlock."
                .into(),
        )),
        BioError::PasscodeNotSet => Err(AppError::Biometric(
            "This device has no passcode set, so it can't be used to unlock TrendWave.".into(),
        )),
        BioError::Unavailable => Err(AppError::Biometric(
            "Biometric authentication isn't available on this device.".into(),
        )),
        other => Err(AppError::Biometric(format!(
            "Biometric authentication failed ({other:?})."
        ))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dismissals_and_failures_stay_locked_quietly() {
        for err in [
            BioError::UserCanceled,
            BioError::AppCanceled,
            BioError::SystemCanceled,
            BioError::Authentication,
            BioError::Exhausted,
            BioError::NotInteractive,
        ] {
            assert!(
                matches!(classify(err), Ok(false)),
                "user-driven dismissal should not surface an error"
            );
        }
    }

    #[test]
    fn configuration_problems_surface_as_errors() {
        for err in [
            BioError::NotEnrolled,
            BioError::PasscodeNotSet,
            BioError::Unavailable,
            BioError::Unknown,
        ] {
            assert!(
                matches!(classify(err), Err(AppError::Biometric(_))),
                "misconfiguration should be reported to the user"
            );
        }
    }
}
