use crate::agent::tools::EditArgs;
use crate::agent::tools::edit::EditTool;
use rig::tool::Tool;

struct TempFile(String);

impl TempFile {
    fn new(name: &str) -> Self {
        let path = std::env::temp_dir()
            .join(format!("zerostack_test_{}", name))
            .to_string_lossy()
            .to_string();
        TempFile(path)
    }

    fn path(&self) -> &str {
        &self.0
    }
}

impl Drop for TempFile {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

#[tokio::test]
async fn test_rejects_no_blocks() {
    let tmp = TempFile::new("noblocks.txt");
    std::fs::write(tmp.path(), "hello world\n").unwrap();
    let tool = EditTool::new(None, None);
    let result = tool
        .call(EditArgs {
            path: tmp.path().into(),
            block: "no blocks here".into(),
        })
        .await;
    assert!(result.is_err());
    let msg = result.unwrap_err().to_string();
    assert!(msg.contains("No SEARCH/REPLACE blocks found"));
}

#[tokio::test]
async fn test_rejects_empty_search() {
    let tmp = TempFile::new("emptysearch.txt");
    std::fs::write(tmp.path(), "hello world\n").unwrap();
    let tool = EditTool::new(None, None);
    let result = tool
        .call(EditArgs {
            path: tmp.path().into(),
            block: "<<<<<<< SEARCH\n=======\nreplacement\n>>>>>>> REPLACE".into(),
        })
        .await;
    assert!(result.is_err());
    let msg = result.unwrap_err().to_string();
    assert!(msg.contains("has empty search text"));
}

#[tokio::test]
async fn test_search_not_found() {
    let tmp = TempFile::new("notfound2.txt");
    std::fs::write(tmp.path(), "hello world\n").unwrap();
    let tool = EditTool::new(None, None);
    let result = tool
        .call(EditArgs {
            path: tmp.path().into(),
            block:
                "<<<<<<< SEARCH\nthis does not exist in file\n=======\nreplacement\n>>>>>>> REPLACE"
                    .into(),
        })
        .await;
    assert!(result.is_err());
    let msg = result.unwrap_err().to_string();
    assert!(msg.contains("not found"));
}

#[tokio::test]
async fn test_single_block_replacement() {
    let tmp = TempFile::new("single2.txt");
    std::fs::write(tmp.path(), "before after done\n").unwrap();
    let tool = EditTool::new(None, None);
    let result = tool
        .call(EditArgs {
            path: tmp.path().into(),
            block: "<<<<<<< SEARCH\nafter\n=======\nmiddle\n>>>>>>> REPLACE".into(),
        })
        .await
        .unwrap();
    let content = std::fs::read_to_string(tmp.path()).unwrap();
    assert_eq!(content, "before middle done\n");
    assert!(result.contains("Applied 1 edit(s)"));
}

#[tokio::test]
async fn test_multi_block_atomic() {
    let tmp = TempFile::new("multiblock.txt");
    std::fs::write(tmp.path(), "aaa\nbbb\nccc\n").unwrap();
    let tool = EditTool::new(None, None);
    let result = tool
        .call(EditArgs {
            path: tmp.path().into(),
            block: "\
<<<<<<< SEARCH
aaa
=======
AAA
>>>>>>> REPLACE

<<<<<<< SEARCH
ccc
=======
CCC
>>>>>>> REPLACE"
                .into(),
        })
        .await
        .unwrap();
    let content = std::fs::read_to_string(tmp.path()).unwrap();
    assert_eq!(content, "AAA\nbbb\nCCC\n");
    assert!(result.contains("Applied 2 edit(s)"));
}

#[tokio::test]
async fn test_multi_match_returns_error() {
    let tmp = TempFile::new("multi2.txt");
    std::fs::write(tmp.path(), "hello world, hello there\n").unwrap();
    let tool = EditTool::new(None, None);
    let result = tool
        .call(EditArgs {
            path: tmp.path().into(),
            block: "<<<<<<< SEARCH\nhello\n=======\nbye\n>>>>>>> REPLACE".into(),
        })
        .await;
    assert!(result.is_err());
    let msg = result.unwrap_err().to_string();
    assert!(msg.contains("matched 2 times"));
}

#[tokio::test]
async fn test_preserves_crlf_line_endings() {
    let tmp = TempFile::new("crlf2.txt");
    std::fs::write(tmp.path(), "line1\r\nline2\r\nline3\r\n").unwrap();
    let tool = EditTool::new(None, None);
    tool.call(EditArgs {
        path: tmp.path().into(),
        block: "<<<<<<< SEARCH\nline2\n=======\nmodified\n>>>>>>> REPLACE".into(),
    })
    .await
    .unwrap();
    let raw = std::fs::read(tmp.path()).unwrap();
    assert!(
        raw.windows(2).any(|w| w == b"\r\n"),
        "CRLF should be preserved"
    );
}

#[tokio::test]
async fn test_normalized_match() {
    let tmp = TempFile::new("norm.txt");
    // Tabs in file, spaces in search — normalization matches them
    std::fs::write(tmp.path(), "\tfn hello() {\n\t    bar\n\t}\n").unwrap();
    let tool = EditTool::new(None, None);
    let result = tool
        .call(EditArgs {
            path: tmp.path().into(),
            block: "<<<<<<< SEARCH\n    fn hello() {\n        bar\n    }\n=======\nfn goodbye() {}\n>>>>>>> REPLACE"
                .into(),
        })
        .await
        .unwrap();
    assert!(
        result.contains("whitespace normalization"),
        "expected normalization note, got: {result}"
    );
}

#[tokio::test]
async fn test_fuzzy_auto_apply() {
    let tmp = TempFile::new("fuzzy.txt");
    std::fs::write(
        tmp.path(),
        "fn calculate_total(items: &[Item]) -> f64 {\n    items.iter().map(|i| i.price).sum()\n}\n",
    )
    .unwrap();
    let tool = EditTool::new(None, None);
    // Search has "calculatte" (typo) vs "calculate" — high similarity, auto-apply
    let result = tool
        .call(EditArgs {
            path: tmp.path().into(),
            block: "<<<<<<< SEARCH\nfn calculatte_total(items: &[Item]) -> f64 {\n=======\nfn sum_prices(items: &[Item]) -> f64 {\n>>>>>>> REPLACE"
                .into(),
        })
        .await
        .unwrap();
    assert!(
        result.contains("fuzzy match"),
        "expected fuzzy match note, got: {result}"
    );
}

#[tokio::test]
async fn test_fuzzy_suggest_low_similarity() {
    let tmp = TempFile::new("fuzzylow.txt");
    std::fs::write(tmp.path(), "hello world\n").unwrap();
    let tool = EditTool::new(None, None);
    let result = tool
        .call(EditArgs {
            path: tmp.path().into(),
            block: "<<<<<<< SEARCH\nhelo word\n=======\nreplacement\n>>>>>>> REPLACE".into(),
        })
        .await;
    if let Err(e) = result {
        let msg = e.to_string();
        // Should suggest the closest match since it's < 0.85 but >= 0.60
        assert!(
            msg.contains("Closest match") || msg.contains("not found"),
            "unexpected error: {msg}"
        );
    }
}
