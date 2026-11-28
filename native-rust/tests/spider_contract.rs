use crayon_core::{
    http::Net,
    spider::{Spider, cache_key, classify_failure, make_url, parse_url},
    storage::Store,
};
use serde_json::{Value, json};
use std::{
    path::{Path, PathBuf},
    process::Command,
    sync::Arc,
};
use tokio_util::sync::CancellationToken;

const EMPTY_JAR: &[u8] = b"PK\x05\x06\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0";

fn jdk_tool(name: &str) -> PathBuf {
    let executable = if cfg!(windows) {
        format!("{name}.exe")
    } else {
        name.to_owned()
    };
    std::env::var_os("JAVA_HOME")
        .map(|home| PathBuf::from(home).join("bin").join(&executable))
        .unwrap_or_else(|| PathBuf::from(executable))
}

fn run_java_tool(command: &mut Command) {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x08000000);
    }
    let output = command.output().expect("JDK 17 tool must be available");
    assert!(
        output.status.success(),
        "Java fixture tool failed: {}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

fn java_fixture(folder: &Path, spider: &Spider) -> PathBuf {
    let source = folder.join("CacheSpider.java");
    let classes = folder.join("classes");
    std::fs::create_dir_all(&classes).unwrap();
    std::fs::write(
        &source,
        r#"package testcase;
import com.github.catvod.crawler.Spider;
public class CacheSpider extends Spider {
    public String homeContent(boolean filter) {
        return "{\"class\":[{\"type_id\":\"1\",\"type_name\":\"Cache fixture\"}],\"list\":[{\"vod_id\":\"fixture\",\"vod_name\":\"Cache fixture\"}]}";
    }
}
"#,
    )
    .unwrap();
    let classpath =
        std::env::join_paths([spider.root.join("out"), spider.root.join("lib/*")]).unwrap();
    run_java_tool(
        Command::new(jdk_tool("javac"))
            .args(["-encoding", "UTF-8", "--release", "8", "-cp"])
            .arg(classpath)
            .arg("-d")
            .arg(&classes)
            .arg(source),
    );
    let jar = folder.join("fixture.jar");
    run_java_tool(
        Command::new(jdk_tool("jar"))
            .args(["--create", "--file"])
            .arg(&jar)
            .arg("-C")
            .arg(classes)
            .arg("."),
    );
    jar
}

fn java_fixture_named(folder: &Path, spider: &Spider, class_name: &str) -> PathBuf {
    let source = folder.join(format!("{class_name}.java"));
    let classes = folder.join(format!("classes-{class_name}"));
    std::fs::create_dir_all(&classes).unwrap();
    let source_text = r#"package testcase;
import com.github.catvod.crawler.Spider;
public class CacheSpider extends Spider {
    public String homeContent(boolean filter) {
        return "{\"class\":[{\"type_id\":\"1\",\"type_name\":\"Cache fixture\"}],\"list\":[{\"vod_id\":\"fixture\",\"vod_name\":\"Cache fixture\"}]}";
    }
}
"#;
    std::fs::write(&source, source_text.replace("CacheSpider", class_name)).unwrap();
    let classpath =
        std::env::join_paths([spider.root.join("out"), spider.root.join("lib/*")]).unwrap();
    run_java_tool(
        Command::new(jdk_tool("javac"))
            .args(["-encoding", "UTF-8", "--release", "8", "-cp"])
            .arg(classpath)
            .arg("-d")
            .arg(&classes)
            .arg(source),
    );
    let jar = folder.join(format!("{class_name}.jar"));
    run_java_tool(
        Command::new(jdk_tool("jar"))
            .args(["--create", "--file"])
            .arg(&jar)
            .arg("-C")
            .arg(classes)
            .arg("."),
    );
    jar
}

fn dex_tools(spider: &Spider) -> PathBuf {
    std::env::var_os("SPIDER_DEX_TOOLS")
        .map(PathBuf::from)
        .into_iter()
        .chain([
            spider.root.join("dex-tools"),
            spider.root.join("dex-tools/dex-tools-v2.4"),
        ])
        .find(|path| path.join("lib").is_dir())
        .expect("dex2jar tools must be available")
}

fn dex_fixture(folder: &Path, spider: &Spider) -> PathBuf {
    let java_jar = java_fixture(folder, spider);
    let dex = folder.join("dex");
    std::fs::create_dir(&dex).unwrap();
    run_java_tool(
        Command::new(spider.java())
            .arg("-cp")
            .arg(dex_tools(spider).join("lib/*"))
            .args(["com.android.dx.command.Main", "--dex"])
            .arg(format!("--output={}", dex.join("classes.dex").display()))
            .arg(java_jar),
    );
    let jar = folder.join("dex-fixture.jar");
    run_java_tool(
        Command::new(jdk_tool("jar"))
            .args(["--create", "--file"])
            .arg(&jar)
            .arg("-C")
            .arg(dex)
            .arg("classes.dex"),
    );
    jar
}

fn dex_fixture_named(folder: &Path, spider: &Spider, class_name: &str) -> PathBuf {
    let java_jar = java_fixture_named(folder, spider, class_name);
    let dex = folder.join(format!("dex-{class_name}"));
    std::fs::create_dir(&dex).unwrap();
    run_java_tool(
        Command::new(spider.java())
            .arg("-cp")
            .arg(dex_tools(spider).join("lib/*"))
            .args(["com.android.dx.command.Main", "--dex"])
            .arg(format!("--output={}", dex.join("classes.dex").display()))
            .arg(java_jar),
    );
    let jar = folder.join(format!("dex-{class_name}.jar"));
    run_java_tool(
        Command::new(jdk_tool("jar"))
            .args(["--create", "--file"])
            .arg(&jar)
            .arg("-C")
            .arg(dex)
            .arg("classes.dex"),
    );
    jar
}

fn fixture_site(jar: &Path) -> Value {
    json!({
        "key": "cache-fixture",
        "sourceId": "contract",
        "jar": jar.to_string_lossy(),
        "spiderClass": "testcase.CacheSpider",
        "ext": ""
    })
}

fn fixture_spider(folder: &Path) -> Spider {
    Spider::new(
        Arc::new(Store::open(folder.join("data")).unwrap()),
        Net::new().unwrap(),
    )
}

async fn fixture_home(spider: &Spider, site: &Value) -> anyhow::Result<Value> {
    spider
        .call(
            site,
            "home",
            json!({"filter": true}),
            5.0,
            &CancellationToken::new(),
        )
        .await
}

#[test]
fn spider_addresses_and_cache_names_preserve_original_ids() {
    let address = make_url("站点:1", "线路/二", "https://a.invalid/play?a=1&b=中文+%");
    assert_eq!(
        parse_url(&address),
        Some((
            "站点:1".into(),
            "线路/二".into(),
            "https://a.invalid/play?a=1&b=中文+%".into()
        ))
    );
    let site = json!({"sourceId":"a","jar":" https://a.invalid/spider.jar "});
    assert_eq!(
        cache_key(&site),
        format!("{:x}", md5::compute("a|https://a.invalid/spider.jar"))
    );
}

#[tokio::test]
async fn real_java_host_supports_concurrent_calls_shutdown_and_restart() {
    let folder = tempfile::tempdir().unwrap();
    let spider = Spider::new(
        Arc::new(Store::open(folder.path()).unwrap()),
        Net::new().unwrap(),
    );
    assert!(spider.runtime_status()["ready"].as_bool().unwrap());
    let (left, right) = tokio::join!(spider.ping(), spider.ping());
    let result = async {
        let left = left?;
        let right = right?;
        assert_eq!(left["sites"], 0);
        assert_eq!(right["sites"], 0);
        assert!(left["maxMb"].as_u64().unwrap() <= 192);
        anyhow::Ok(())
    }
    .await;
    spider.stop().await.unwrap();
    result.unwrap();
    assert_eq!(spider.ping().await.unwrap()["sites"], 0);
    spider.stop().await.unwrap();
}

#[tokio::test]
async fn real_java_spider_jar_is_opened_and_called() {
    let folder = tempfile::tempdir().unwrap();
    let spider = fixture_spider(folder.path());
    let jar = java_fixture(folder.path(), &spider);
    let result = fixture_home(&spider, &fixture_site(&jar)).await;
    spider.stop().await.unwrap();
    let home = result.expect("a real JVM class JAR should open successfully");
    assert_eq!(home["list"][0]["vod_id"], "fixture");
    assert_eq!(home["class"][0]["type_name"], "Cache fixture");
}

#[tokio::test]
async fn empty_converted_cache_is_recovered_from_valid_raw_dex() {
    let folder = tempfile::tempdir().unwrap();
    let spider = fixture_spider(folder.path());
    let jar = dex_fixture(folder.path(), &spider);
    let site = fixture_site(&jar);
    let cache = spider.store.root.join("spider-cache");
    std::fs::create_dir_all(&cache).unwrap();
    let key = cache_key(&site);
    let converted = cache.join(format!("{key}.jar"));
    let raw = cache.join(format!("{key}.raw.zip"));
    let original = std::fs::read(&jar).unwrap();
    std::fs::write(&raw, &original).unwrap();
    std::fs::write(&converted, EMPTY_JAR).unwrap();
    // Recovery must use the valid cached input even if its original source is gone.
    std::fs::remove_file(jar).unwrap();

    let result = fixture_home(&spider, &site).await;
    spider.stop().await.unwrap();
    let home = result.expect("an empty converted JAR must not poison the cache");
    assert_eq!(home["list"][0]["vod_id"], "fixture");
    assert_eq!(std::fs::read(raw).unwrap(), original);
    assert_ne!(
        std::fs::read(converted).ok().as_deref(),
        Some(EMPTY_JAR),
        "empty conversion output must be removed or replaced"
    );
}

#[tokio::test]
async fn dex_conversion_preserves_existing_jvm_cache_for_a_different_class() {
    let folder = tempfile::tempdir().unwrap();
    let spider = fixture_spider(folder.path());
    let old_jar = java_fixture(folder.path(), &spider);
    let raw_dex = dex_fixture_named(folder.path(), &spider, "UpdatedSpider");
    let mut site = fixture_site(&raw_dex);
    site["spiderClass"] = json!("testcase.UpdatedSpider");
    let key = cache_key(&site);
    let cache = spider.store.root.join("spider-cache");
    std::fs::create_dir_all(&cache).unwrap();
    let shared = cache.join(format!("{key}.jar"));
    let raw = cache.join(format!("{key}.raw.zip"));
    let old_bytes = std::fs::read(&old_jar).unwrap();
    std::fs::write(&shared, &old_bytes).unwrap();
    std::fs::copy(&raw_dex, &raw).unwrap();

    let mut old_site = site.clone();
    old_site["spiderClass"] = json!("testcase.CacheSpider");
    let old_home = fixture_home(&spider, &old_site).await;
    let new_home = fixture_home(&spider, &site).await;
    let shared_after = std::fs::read(&shared).unwrap();
    let updated_jars = std::fs::read_dir(&cache)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .filter(|path| path.extension().is_some_and(|extension| extension == "jar"))
        .collect::<Vec<_>>();
    let cache_info = spider
        .cache_info(&[site.clone(), old_site.clone()])
        .await
        .unwrap();
    let reported_bytes = cache_info["sites"][0]["bytes"].as_u64().unwrap();
    let actual_bytes = std::fs::read_dir(&cache)
        .unwrap()
        .map(|entry| entry.unwrap().metadata().unwrap().len())
        .sum::<u64>();
    let old_again = fixture_home(&spider, &old_site).await;
    spider.stop().await.unwrap();

    assert_eq!(old_home.unwrap()["list"][0]["vod_id"], "fixture");
    assert_eq!(new_home.unwrap()["list"][0]["vod_id"], "fixture");
    assert_eq!(
        shared_after, old_bytes,
        "conversion must keep the shared JAR immutable"
    );
    assert_eq!(
        updated_jars.len(),
        2,
        "the converted class needs its own cache artifact"
    );
    assert_eq!(
        reported_bytes, actual_bytes,
        "cache accounting must include class-specific JARs"
    );
    assert_eq!(old_again.unwrap()["list"][0]["vod_id"], "fixture");
}

#[tokio::test]
async fn missing_class_does_not_replace_an_open_shared_cache_jar() {
    let folder = tempfile::tempdir().unwrap();
    let spider = fixture_spider(folder.path());
    let jar = dex_fixture(folder.path(), &spider);
    let site = fixture_site(&jar);
    let cache = spider.store.root.join("spider-cache");
    std::fs::create_dir_all(&cache).unwrap();
    let key = cache_key(&site);
    let converted = cache.join(format!("{key}.jar"));
    let raw = cache.join(format!("{key}.raw.zip"));
    let original = std::fs::read(folder.path().join("fixture.jar")).unwrap();
    std::fs::write(&converted, &original).unwrap();
    std::fs::copy(jar, &raw).unwrap();

    let first = fixture_home(&spider, &site).await;
    let mut missing_site = site.clone();
    missing_site["spiderClass"] = json!("testcase.MissingSpider");
    let mut attempts = Vec::new();
    for _ in 0..2 {
        let missing = fixture_home(&spider, &missing_site).await;
        let bytes = std::fs::read(&converted);
        let again = fixture_home(&spider, &site).await;
        let files = std::fs::read_dir(&cache).and_then(|entries| {
            entries
                .map(|entry| entry.map(|entry| entry.file_name().to_string_lossy().into_owned()))
                .collect::<std::io::Result<Vec<_>>>()
        });
        attempts.push((missing, bytes, again, files));
    }
    spider.stop().await.unwrap();

    let home = first.expect("the existing shared JAR must open before requesting a missing class");
    assert_eq!(home["list"][0]["vod_id"], "fixture");
    for (missing, bytes, again, files) in attempts {
        let error = missing.expect_err("a missing class must be rejected");
        assert_eq!(classify_failure(&error.to_string()), "missing_class");
        assert_eq!(
            bytes.expect("a missing class must preserve the open shared JAR"),
            original,
            "a missing class must not replace a shared JAR"
        );
        let home = again.expect("the original Spider must remain callable after a missing class");
        assert_eq!(home["list"][0]["vod_id"], "fixture");
        let mut files = files.expect("the shared cache directory must remain readable");
        files.sort();
        assert_eq!(
            files,
            [format!("{key}.jar"), format!("{key}.raw.zip")],
            "a missing class must not leave a partial conversion"
        );
    }
}

#[tokio::test]
async fn conversion_without_jvm_classes_is_rejected_without_poisoning_cache() {
    let folder = tempfile::tempdir().unwrap();
    let spider = fixture_spider(folder.path());
    let source = folder.path().join("empty-dex");
    std::fs::create_dir(&source).unwrap();
    let generator = folder.path().join("EmptyDex.java");
    std::fs::write(
        &generator,
        r#"import java.nio.file.*;
import com.googlecode.d2j.dex.writer.DexFileWriter;
public class EmptyDex {
    public static void main(String[] args) throws Exception {
        DexFileWriter writer = new DexFileWriter();
        writer.visitEnd();
        Files.write(Path.of(args[0]), writer.toByteArray());
    }
}
"#,
    )
    .unwrap();
    run_java_tool(
        Command::new(jdk_tool("java"))
            .arg("-cp")
            .arg(dex_tools(&spider).join("lib/*"))
            .arg(generator)
            .arg(source.join("classes.dex")),
    );
    let jar = folder.path().join("empty-dex.jar");
    run_java_tool(
        Command::new(jdk_tool("jar"))
            .args(["--create", "--file"])
            .arg(&jar)
            .arg("-C")
            .arg(source)
            .arg("classes.dex"),
    );
    let site = fixture_site(&jar);
    let key = cache_key(&site);
    let cache = spider.store.root.join("spider-cache");
    let mut attempts = Vec::new();
    for _ in 0..2 {
        let result = fixture_home(&spider, &site).await;
        let files = std::fs::read_dir(&cache)
            .unwrap()
            .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
            .collect::<Vec<_>>();
        attempts.push((result, files));
    }
    spider.stop().await.unwrap();

    for (result, files) in attempts {
        let error = result.expect_err("DEX without classes cannot produce a usable Spider JAR");
        assert!(
            !error.to_string().contains("ClassNotFoundException"),
            "invalid conversion must be rejected before opening a Spider: {error:#}"
        );
        assert!(
            files.iter().all(|name| name == &format!("{key}.raw.zip")),
            "failed conversion left a final or partial JAR in cache: {files:?}"
        );
    }
}
