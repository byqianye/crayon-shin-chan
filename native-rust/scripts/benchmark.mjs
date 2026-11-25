import assert from 'node:assert/strict';
import { createHash } from 'node:crypto';
import { spawn, spawnSync } from 'node:child_process';
import { mkdtemp, mkdir, rm, writeFile } from 'node:fs/promises';
import { createServer } from 'node:http';
import { createRequire } from 'node:module';
import { tmpdir } from 'node:os';
import { dirname, join, resolve } from 'node:path';
import { fileURLToPath, pathToFileURL } from 'node:url';

const root = resolve(dirname(fileURLToPath(import.meta.url)), '../..');
const folder = await mkdtemp(join(tmpdir(), 'crayon-rust-benchmark-'));
const digest = (value) => createHash('sha256').update(JSON.stringify(value)).digest('hex');
const median = (values) => {
  const measured = values.slice(1).sort((a, b) => a - b);
  return measured[Math.floor(measured.length / 2)];
};
const summarize = (value) => ({
  searchFirstMs: median(value.search.map((v) => v.first)),
  searchTotalMs: median(value.search.map((v) => v.total)),
  mergeUnknownYearMs: median(value.merge[0].ms),
  mergeKnownYearMs: median(value.merge[1].ms)
});
const server = createServer((_request, response) => {
  setTimeout(() => {
    response.setHeader('content-type', 'application/json');
    response.end(JSON.stringify({
      code: 1, page: 1, pagecount: 1,
      list: Array.from({ length: 100 }, (_, index) => ({
        vod_id: String(index), vod_name: '作品编号' + index, vod_pic: '',
        vod_year: '2026', type_name: '电影', vod_content: '测试',
        vod_play_from: 'line', vod_play_url: '第一集$https://example.invalid/a.mp4'
      }))
    }));
  }, 25);
});
let baseline;
try {
  const build = spawnSync('cargo', [
    'build', '--locked', '--release', '--no-default-features',
    '--manifest-path', join(root, 'native-rust/Cargo.toml'),
    '--example', 'service_benchmark', '--message-format=json-render-diagnostics'
  ], { encoding: 'utf8', windowsHide: true });
  if (build.stderr) process.stderr.write(build.stderr);
  const events = (build.stdout || '').trim().split(String.fromCharCode(10)).filter(Boolean).map((line) => JSON.parse(line));
  for (const event of events) {
    if (event.reason === 'compiler-message' && event.message.rendered) process.stderr.write(event.message.rendered);
  }
  assert.ifError(build.error);
  assert.equal(build.status, 0, 'Rust benchmark build failed');
  const artifacts = events.filter((v) => v.reason === 'compiler-artifact' && v.target.name === 'service_benchmark' && v.executable);
  assert.equal(artifacts.length, 1, 'Expected one benchmark executable');

  const { esbuildExe } = await import(pathToFileURL(join(root, 'scripts/lib/spider-bundle.mjs')));
  const data = join(folder, 'baseline-data');
  await mkdir(data);
  const stub = join(folder, 'electron.cjs');
  const entry = join(folder, 'baseline.ts');
  const bundle = join(folder, 'baseline.cjs');
  await writeFile(stub, 'module.exports={app:{isPackaged:false,getAppPath:()=>'
    + JSON.stringify(root) + ',getPath:()=>'+ JSON.stringify(data) + '}}');
  const exports = ['search', 'sources', 'store'].map((name) => 'export * as ' + name + ' from '
    + JSON.stringify(join(root, 'src/main/services', name + '.ts')));
  exports.push('export * as merge from ' + JSON.stringify(join(root, 'src/core/dedupe/merge.ts')));
  await writeFile(entry, exports.join(String.fromCharCode(10)));
  const bundleBuild = spawnSync(esbuildExe(), [entry, '--bundle', '--platform=node', '--format=cjs',
    '--outfile=' + bundle, '--alias:electron=' + stub,
    '--tsconfig=' + join(root, 'tsconfig.node.json'), '--log-level=error'
  ], { stdio: 'inherit', windowsHide: true });
  assert.ifError(bundleBuild.error);
  assert.equal(bundleBuild.status, 0, 'Original service bundle failed');

  await new Promise((accept) => server.listen(0, '127.0.0.1', accept));
  const base = 'http://127.0.0.1:' + server.address().port;
  const sites = Array.from({ length: 32 }, (_, index) => ({
    key: 'site-' + index, name: '站点' + index, type: 3,
    api: base + '/cms?site=' + index, enabled: true, searchable: true
  }));
  const sources = [{ id: 'fixture', name: 'fixture', url: base + '/config', enabled: true, kind: 'url', raw: { sites } }];
  const merge = [{ list: Array.from({ length: 1200 }, (_, index) => ({
    vod_id: String(index), vod_name: '作品编号' + index, siteKey: String(index % 8),
    type_name: '电影', vod_year: '', vod_pic: '', vod_remarks: ''
  })) }];
  baseline = createRequire(import.meta.url)(bundle);
  baseline.store.writeStore('sources', sources);
  baseline.store.writeStore('settings', { concurrency: 8, timeout: 5 });
  await baseline.sources.hydrate();

  const search = [];
  for (let round = 0; round < 10; round++) {
    let first;
    const started = performance.now();
    const output = await baseline.search.searchAll('fixture-' + round, {}, () => { first ??= performance.now() - started; });
    const total = performance.now() - started;
    assert.equal(output.results.length, 32);
    assert(output.results.every((v) => v.ok));
    const identity = output.results.flatMap((v) => v.list).map((v) => [v.siteKey, v.vod_id, v.vod_name]);
    identity.sort((a, b) => {
      const left = JSON.stringify(a), right = JSON.stringify(b);
      return left < right ? -1 : left > right ? 1 : 0;
    });
    assert.equal(identity.length, 960);
    search.push({ first, total, digest: digest(identity) });
  }
  const merges = [];
  for (const knownYear of [false, true]) {
    const input = structuredClone(merge);
    if (knownYear) for (const vod of input[0].list) vod.vod_year = '2026';
    const ms = [];
    let expected;
    for (let round = 0; round < 10; round++) {
      const started = performance.now();
      const output = baseline.merge.mergeResults(input);
      ms.push(performance.now() - started);
      assert.equal(output.merged.length, 1200);
      const current = digest(output.merged.map((v) => [v.siteKey, v.vod_id, v.vod_name, v.allSiteKeys, v.alternatives]));
      assert(!expected || expected === current, 'Original merge was unstable');
      expected = current;
    }
    merges.push({ knownYear, ms, digest: expected });
  }

  const item = (name, site, year = '', type = '') => ({
    vod_id: site + ':' + name, siteKey: site, vod_name: name, vod_year: year,
    type_name: type, vod_pic: '', vod_remarks: ''
  });
  const contractInputs = [
    [item('庆余年第二季[全36集]', 'a', '2024', '国产剧'), item('庆余年 第二季', 'b'),
      item('庆余年第一季', 'c', '2019', '国产剧'), item('庆余年第二季[电影解说]', 'd')],
    [item('射雕英雄传', 'a', '1983', '电视剧'), item('射雕英雄传', 'b', '2017', '电视剧'),
      item('射雕英雄传', 'c'), item('射雕英雄传', 'd', '2017', '电影')],
    [item('阿凡达', 'a'), item('北京爱情故事', 'b'), item('重庆森林', 'c'), item('影视A', 'd'), item('影视a', 'e')],
    [item('长标题测试作品编号甲乙丙丁戊己庚辛', 'a', '2026'), item('长标题测试作品编号甲乙丙丁戊己庚壬', 'b', '2026')],
    [{ ...item('同片', 'a'), vod_id: 'sparse' }, { ...item('同片高清', 'a'), vod_id: 'rich-long', vod_pic: 'poster' },
      { ...item('同片', 'a'), vod_id: 'rich-short', vod_pic: 'poster' }, item('同片', 'b')]
  ];
  const contracts = contractInputs.map((list, index) => {
    const results = [{ list }], keyword = index === 0 ? '庆余年' : '';
    return { results, keyword, expected: baseline.merge.mergeResults(results, { keyword }) };
  });
  const scenario = join(folder, 'scenario.json');
  await writeFile(scenario, JSON.stringify({ sites: 32, items: 960, sources, merge, contracts }));
  const child = spawn(artifacts[0].executable, [scenario, join(folder, 'rust-data')], { windowsHide: true });
  let output = '', error = '';
  child.stdout.on('data', (part) => { output += part; });
  child.stderr.on('data', (part) => { error += part; });
  const code = await new Promise((accept, reject) => { child.once('error', reject); child.once('exit', accept); });
  assert.equal(code, 0, error || 'Rust benchmark failed');
  const rust = JSON.parse(output.trim());
  assert.deepEqual(rust.search.map((v) => v.digest), search.map((v) => v.digest), 'Search results differ');
  assert.deepEqual(rust.merge.map((v) => v.digest), merges.map((v) => v.digest), 'Merge order or alternatives differ');
  assert.equal(rust.contracts, contracts.length);
  console.log('[PASS] search identities, merge ordering, alternatives and compatibility fixtures match');
  console.log(JSON.stringify({
    rounds: 10, warmup: 1, sites: 32, concurrency: 8, delayMs: 25, itemsPerSite: 30,
    mergeItems: 1200, identicalResults: true, compatibilityFixtures: contracts.length,
    baseline: summarize({ search, merge: merges }), rust: summarize(rust),
    componentsMs: Object.fromEntries(Object.entries(rust.components).map(([key, values]) => [key, median(values)]))
  }, null, 2));
} finally {
  try {
    await baseline?.store.flushStore();
  } finally {
    if (server.listening) await new Promise((accept) => server.close(accept));
    await rm(folder, { recursive: true, force: true });
  }
}
