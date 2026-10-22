//! Keep command formatting separate from collection and rule evaluation.

use super::PageBudget;
use crate::{FrameworkError, console};
use suprnova_macros::command;
use tokio::io::AsyncReadExt;

#[command(
    name = "budget:audit",
    description = "Audit a page URL or HAR against the page budget"
)]
async fn audit(args: Vec<String>) -> Result<(), FrameworkError> {
    let [target] = args.as_slice() else {
        return Err(FrameworkError::bad_request(
            "usage: budget:audit <url | path.har>",
        ));
    };
    let budget = PageBudget::from_env()?;
    let audit = if target.starts_with("http://") || target.starts_with("https://") {
        budget.audit_url(target).await?
    } else {
        let metadata = tokio::fs::metadata(target)
            .await
            .map_err(|e| FrameworkError::bad_request(format!("page budget HAR {target}: {e}")))?;
        // Headers and encoded bodies can be larger than response payloads.
        let limit = budget.max_response_bytes.saturating_mul(4) as u64;
        if metadata.len() > limit {
            return Err(FrameworkError::bad_request(format!(
                "page budget HAR exceeds {limit} bytes"
            )));
        }
        let file = tokio::fs::File::open(target)
            .await
            .map_err(|e| FrameworkError::bad_request(format!("page budget HAR {target}: {e}")))?;
        let mut json = String::new();
        file.take(limit.saturating_add(1))
            .read_to_string(&mut json)
            .await
            .map_err(|e| FrameworkError::bad_request(format!("page budget HAR {target}: {e}")))?;
        if json.len() as u64 > limit {
            return Err(FrameworkError::bad_request(format!(
                "page budget HAR exceeds {limit} bytes"
            )));
        }
        tokio::task::spawn_blocking(move || budget.audit_har(&json))
            .await
            .map_err(|e| FrameworkError::internal(format!("page budget HAR evaluator: {e}")))??
    };
    console::line(&audit);
    if audit.findings.is_empty() {
        Ok(())
    } else {
        Err(FrameworkError::bad_request(format!(
            "page budget failed: {} findings",
            audit.findings.len()
        )))
    }
}
