/**
 * Spider 契约测试：不依赖外网，用一组合成 Spider 覆盖
 * 缺类、类型不对、Android API 缺失、native 加固、超时、TLS、空播放字段、
 * 非法 JSON 和正常全链路。每次 CI 都跑，真实源矩阵走定时任务。
 *
 * 需要 JDK 17（JAVA_HOME），并先构建 spider-host/out。
 */
import { existsSync } from 'node:fs'
import { mkdir, mkdtemp, readFile, rm, writeFile } from 'node:fs/promises'
import { tmpdir } from 'node:os'
import { delimiter, join, resolve } from 'node:path'
import { spawnSync } from 'node:child_process'
import { projectRoot } from './lib/spider-bundle.mjs'

const root = projectRoot()
const javaHome = process.env.JAVA_HOME || ''
if (!javaHome) throw new Error('请设置 JAVA_HOME 指向 JDK 17')

const javac = join(javaHome, 'bin', 'javac.exe')
const jarTool = join(javaHome, 'bin', 'jar.exe')
if (!existsSync(javac) || !existsSync(jarTool)) throw new Error(`JAVA_HOME 下缺少 javac/jar：${javaHome}`)
if (!existsSync(join(root, 'spider-host', 'out', 'host', 'SpiderHost.class'))) {
  throw new Error('缺少 spider-host/out，请先运行 pnpm spider:build')
}

const temp = await mkdtemp(join(tmpdir(), 'crayon-spider-contract-'))
try {
  const srcDir = join(temp, 'src', 'testcase')
  const classesDir = join(temp, 'classes')
  await mkdir(srcDir, { recursive: true })
  await mkdir(classesDir, { recursive: true })

  const files = {
    'NormalSpider.java': `package testcase;
import com.github.catvod.crawler.Spider;
import java.util.List;
public class NormalSpider extends Spider {
  public String homeContent(boolean filter) {
    return "{\\"class\\":[{\\"type_id\\":\\"1\\",\\"type_name\\":\\"电影\\"}],\\"list\\":[{\\"vod_id\\":\\"1\\",\\"vod_name\\":\\"契约测试片\\"}]}";
  }
  public String searchContent(String key, boolean quick) {
    return "{\\"list\\":[{\\"vod_id\\":\\"1\\",\\"vod_name\\":\\"契约测试片\\"}]}";
  }
  public String detailContent(List<String> ids) {
    return "{\\"list\\":[{\\"vod_id\\":\\"1\\",\\"vod_name\\":\\"契约测试片\\",\\"vod_play_from\\":\\"线路1\\",\\"vod_play_url\\":\\"第1集$http://example.com/a.m3u8\\"}]}";
  }
  public String playerContent(String flag, String id, List<String> vipFlags) {
    return "{\\"url\\":\\"http://example.com/a.m3u8\\"}";
  }
}`,
    'NotSpider.java': `package testcase;
public class NotSpider {
  public NotSpider() {}
}`,
    'AndroidApiSpider.java': `package testcase;
import com.github.catvod.crawler.Spider;
public class AndroidApiSpider extends Spider {
  public void init(android.content.Context context, String extend) {
    throw new NoSuchMethodError("android.content.Context.getExternalFilesDir");
  }
}`,
    'NativeSpider.java': `package testcase;
import com.github.catvod.crawler.Spider;
public class NativeSpider extends Spider {
  public void init(android.content.Context context, String extend) {
    throw new UnsatisfiedLinkError("dlopen failed: libnative.so");
  }
}`,
    'TimeoutSpider.java': `package testcase;
import com.github.catvod.crawler.Spider;
public class TimeoutSpider extends Spider {
  public String searchContent(String key, boolean quick) {
    try { Thread.sleep(60000L); } catch (InterruptedException ignored) {}
    return "{\\"list\\":[]}";
  }
}`,
    'TlsSpider.java': `package testcase;
import com.github.catvod.crawler.Spider;
public class TlsSpider extends Spider {
  public String searchContent(String key, boolean quick) {
    throw new RuntimeException(new javax.net.ssl.SSLHandshakeException("certificate verify failed"));
  }
}`,
    'EmptyPlaySpider.java': `package testcase;
import com.github.catvod.crawler.Spider;
import java.util.List;
public class EmptyPlaySpider extends Spider {
  public String searchContent(String key, boolean quick) {
    return "{\\"list\\":[{\\"vod_id\\":\\"1\\",\\"vod_name\\":\\"空播放字段\\"}]}";
  }
  public String detailContent(List<String> ids) {
    return "{\\"list\\":[{\\"vod_id\\":\\"1\\",\\"vod_name\\":\\"空播放字段\\",\\"vod_play_from\\":\\"\\",\\"vod_play_url\\":\\"\\"}]}";
  }
  public String playerContent(String flag, String id, List<String> vipFlags) {
    return "{}";
  }
}`,
    'InvalidJsonSpider.java': `package testcase;
import com.github.catvod.crawler.Spider;
public class InvalidJsonSpider extends Spider {
  public String searchContent(String key, boolean quick) {
    return "not-json";
  }
}`,
    'MissingMethodsSpider.java': `package testcase;
import com.github.catvod.crawler.Spider;
public class MissingMethodsSpider extends Spider {
}`
  }
  for (const [name, source] of Object.entries(files)) {
    await writeFile(join(srcDir, name), source, 'utf8')
  }

  const classpath = [
    join(root, 'spider-host', 'out'),
    join(root, 'spider-host', 'lib', '*')
  ].join(delimiter)
  const sourceFiles = Object.keys(files).map((name) => join(srcDir, name))
  const compile = spawnSync(
    javac,
    ['-encoding', 'UTF-8', '-source', '17', '-target', '17', '-cp', classpath, '-d', classesDir, ...sourceFiles],
    { stdio: 'inherit' }
  )
  if (compile.status !== 0) throw new Error(`javac 失败（${compile.status}）`)

  const testJar = join(temp, 'spider-contract.jar')
  const pack = spawnSync(jarTool, ['--create', '--file', testJar, '-C', classesDir, '.'], {
    stdio: 'inherit'
  })
  if (pack.status !== 0) throw new Error(`jar 打包失败（${pack.status}）`)

  const cases = {
    keyword: '契约测试',
    timeout: 5,
    jar: testJar,
    cases: [
      { key: 'normal', name: '正常全链路', className: 'testcase.NormalSpider', expect: 'usable' },
      { key: 'missing-class', name: '缺类', className: 'testcase.MissingClass', expect: 'missing_class' },
      { key: 'not-spider', name: '不是 Spider 子类', className: 'testcase.NotSpider', expect: 'wrong_type' },
      { key: 'android-api', name: 'Android API 缺失', className: 'testcase.AndroidApiSpider', expect: 'android_api' },
      { key: 'native', name: 'native 加固', className: 'testcase.NativeSpider', expect: 'native' },
      { key: 'timeout', name: '调用超时', className: 'testcase.TimeoutSpider', expect: 'timeout' },
      { key: 'tls', name: 'TLS 证书失败', className: 'testcase.TlsSpider', expect: 'tls' },
      { key: 'empty-play', name: '空播放字段', className: 'testcase.EmptyPlaySpider', expect: 'empty_play_fields' },
      { key: 'invalid-json', name: '非法 JSON', className: 'testcase.InvalidJsonSpider', expect: 'invalid_json' },
      { key: 'missing-methods', name: '缺方法实现', className: 'testcase.MissingMethodsSpider', expect: 'empty_search' }
    ]
  }
  const casesFile = join(temp, 'cases.json')
  const reportDir = join(temp, 'report')
  await writeFile(casesFile, JSON.stringify(cases, null, 2), 'utf8')

  const run = spawnSync(
    process.execPath,
    [
      join(root, 'scripts', 'spider-matrix.mjs'),
      '--cases',
      casesFile,
      '--out',
      reportDir,
      '--keyword',
      cases.keyword,
      '--timeout',
      String(cases.timeout)
    ],
    { stdio: 'inherit' }
  )
  if (run.status !== 0) throw new Error(`契约矩阵执行失败（${run.status}）`)

  const report = JSON.parse(await readFile(join(reportDir, 'spider-matrix.json'), 'utf8'))
  const actual = new Map(report.results.map((item) => [item.key, item.category]))
  const failures = cases.cases.filter((item) => actual.get(item.key) !== item.expect)
  console.log('\n[spider-contract] 分类结果：')
  for (const item of cases.cases) {
    const got = actual.get(item.key) ?? '(missing)'
    console.log(`  ${got === item.expect ? '✓' : '✗'} ${item.key.padEnd(16)} ${got} (期望 ${item.expect})`)
  }
  if (failures.length) {
    throw new Error(`契约分类不一致：${failures.map((item) => item.key).join('、')}`)
  }
  console.log(`[spider-contract] ${cases.cases.length}/${cases.cases.length} 通过`)
} finally {
  try {
    await rm(temp, { recursive: true, force: true })
  } catch {
    // 临时目录由系统回收
  }
}
