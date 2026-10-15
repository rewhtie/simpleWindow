const { execFileSync, spawn } = require('child_process');
const { existsSync, readFileSync, readdirSync, realpathSync, statSync, writeFileSync } = require('fs');
const { basename, join, resolve } = require('path');
const readline = require('readline');

const ROOT = resolve(__dirname, '..');
const TOKEN_FILE = join(ROOT, '.release-token');

function ask(question, defaultValue = '') {
  return new Promise((resolveAnswer) => {
    const input = readline.createInterface({ input: process.stdin, output: process.stdout });
    const suffix = defaultValue ? ` (${defaultValue})` : '';
    input.question(`${question}${suffix}: `, (answer) => {
      input.close();
      resolveAnswer(answer.trim() || defaultValue);
    });
  });
}

function run(command, args, options = {}) {
  return new Promise((resolveRun, rejectRun) => {
    const child = spawn(command, args, { cwd: ROOT, stdio: 'inherit', shell: true, ...options });
    child.once('error', rejectRun);
    child.once('exit', (code) => {
      if (code === 0) resolveRun();
      else rejectRun(new Error(`${command} 退出码 ${code}`));
    });
  });
}

function projectConfig() {
  const path = join(ROOT, 'src-tauri', 'tauri.conf.json');
  return JSON.parse(readFileSync(path, 'utf8'));
}

function productName() {
  return projectConfig().productName || 'simpleWindow';
}

function releaseTag() {
  const version = projectConfig().version;
  if (!version || typeof version !== 'string') {
    throw new Error('tauri.conf.json 中缺少有效的 version。');
  }
  return version.startsWith('v') ? version : `v${version}`;
}

function tauriMsiDirectory() {
  return join(ROOT, 'src-tauri', 'target', 'release', 'bundle', 'msi');
}

function listMsiFiles(directory = tauriMsiDirectory()) {
  if (!existsSync(directory)) return [];
  return readdirSync(directory)
    .filter((name) => /\.msi$/i.test(name) && statSync(join(directory, name)).isFile())
    .map((name) => join(directory, name));
}

function getToken() {
  if (process.env.GH_TOKEN) return process.env.GH_TOKEN.trim();
  if (process.env.GITHUB_TOKEN) return process.env.GITHUB_TOKEN.trim();
  if (existsSync(TOKEN_FILE)) return readFileSync(TOKEN_FILE, 'utf8').trim();
  return null;
}

async function promptAndSaveToken(message) {
  console.log(`\n${message}`);
  console.log('请创建能够向目标仓库写入 Release 的 GitHub Token。\n');
  const token = (await ask('请输入新的 GitHub Token')).trim();
  if (!token) throw new Error('未提供 GitHub Token。');

  try {
    writeFileSync(TOKEN_FILE, `${token}\n`, { mode: 0o600 });
    console.log('Token 已保存到本地 .release-token。\n');
  } catch (error) {
    console.warn(`Token 保存失败，本次仍会继续：${error.message}`);
  }
  return token;
}

async function validateToken(token, owner, repo) {
  return fetch(
    `https://api.github.com/repos/${encodeURIComponent(owner)}/${encodeURIComponent(repo)}`,
    { headers: authHeaders(token) },
  );
}

async function ensureToken(owner, repo) {
  let token = getToken();
  let source = process.env.GH_TOKEN || process.env.GITHUB_TOKEN ? '环境变量' : '.release-token';

  if (!token) {
    token = await promptAndSaveToken('未找到 GH_TOKEN、GITHUB_TOKEN 或有效的 .release-token。');
    source = '新输入';
  }

  while (true) {
    const response = await validateToken(token, owner, repo);
    if (response.ok) return token;

    if (response.status !== 401) {
      throw new Error(`验证 GitHub Token 失败：${await responseError(response)}`);
    }

    writeFileSync(TOKEN_FILE, '');
    console.warn(`${source}中的 GitHub Token 无效或已过期，本地缓存已清空。`);
    token = await promptAndSaveToken('请重新输入有效的 GitHub Token。');
    source = '新输入';
  }
}

function githubRemote() {
  let topLevel;
  try {
    topLevel = execFileSync('git', ['-C', ROOT, 'rev-parse', '--show-toplevel'], {
      encoding: 'utf8',
      stdio: ['ignore', 'pipe', 'ignore'],
    }).trim();
  } catch {
    throw new Error('当前项目尚未初始化为 Git 仓库。请先初始化并配置 GitHub origin。');
  }

  if (realpathSync.native(topLevel) !== realpathSync.native(ROOT)) {
    throw new Error('simpleWindow 不是独立 Git 仓库；已拒绝使用上层仓库作为发布目标。');
  }

  let remote;
  try {
    remote = execFileSync('git', ['-C', ROOT, 'remote', 'get-url', 'origin'], {
      encoding: 'utf8',
      stdio: ['ignore', 'pipe', 'ignore'],
    }).trim();
  } catch {
    throw new Error('当前仓库没有 origin。请先将 origin 设置为目标 GitHub 仓库。');
  }

  const match = remote.match(/^https?:\/\/github\.com\/([^/]+)\/([^/]+?)(?:\.git)?$/i)
    || remote.match(/^git@github\.com:([^/]+)\/([^/]+?)(?:\.git)?$/i)
    || remote.match(/^ssh:\/\/git@github\.com\/([^/]+)\/([^/]+?)(?:\.git)?$/i);
  if (!match) throw new Error('origin 不是可识别的 GitHub 仓库地址，已停止发布。');
  return { owner: match[1], repo: match[2] };
}

function authHeaders(token) {
  return {
    Authorization: `Bearer ${token}`,
    Accept: 'application/vnd.github+json',
    'X-GitHub-Api-Version': '2022-11-28',
    'User-Agent': 'simple-window-release',
  };
}

async function responseError(response) {
  const detail = await response.text();
  if (response.status === 401) return 'GitHub Token 无效或已过期。';
  if (response.status === 403) return 'GitHub 拒绝了请求，请检查 Token 的仓库权限。';
  if (response.status === 404) return '找不到目标 GitHub 仓库，或 Token 无权访问。';
  return `${response.status} ${detail}`;
}

async function findOrCreateRelease(token, owner, repo, tag, notes) {
  const repository = `${encodeURIComponent(owner)}/${encodeURIComponent(repo)}`;
  const headers = authHeaders(token);
  const existing = await fetch(
    `https://api.github.com/repos/${repository}/releases/tags/${encodeURIComponent(tag)}`,
    { headers },
  );

  if (existing.ok) {
    console.log(`[release] 复用已有 Release ${tag}。`);
    return existing.json();
  }
  if (existing.status !== 404) {
    throw new Error(`查询 Release 失败：${await responseError(existing)}`);
  }

  const created = await fetch(`https://api.github.com/repos/${repository}/releases`, {
    method: 'POST',
    headers: { ...headers, 'Content-Type': 'application/json' },
    body: JSON.stringify({
      tag_name: tag,
      name: tag,
      body: notes,
      draft: false,
      prerelease: false,
    }),
  });
  if (!created.ok) throw new Error(`创建 Release 失败：${await responseError(created)}`);
  console.log(`[release] 已创建 Release ${tag}。`);
  return created.json();
}

async function allAssets(token, assetsUrl) {
  const assets = [];
  for (let page = 1; ; page += 1) {
    const response = await fetch(`${assetsUrl}?per_page=100&page=${page}`, {
      headers: authHeaders(token),
    });
    if (!response.ok) throw new Error(`读取 Release 附件失败：${await responseError(response)}`);
    const batch = await response.json();
    assets.push(...batch);
    if (batch.length < 100) return assets;
  }
}

async function uploadFiles(token, release, files) {
  const uploadUrl = release.upload_url.replace('{?name,label}', '');
  const assets = await allAssets(token, release.assets_url);

  for (const file of files) {
    const name = basename(file);
    const duplicate = assets.find((asset) => asset.name === name);
    if (duplicate) {
      console.log(`[release] 删除同名附件 ${name}。`);
      const deleted = await fetch(duplicate.url, { method: 'DELETE', headers: authHeaders(token) });
      if (!deleted.ok) throw new Error(`删除旧附件 ${name} 失败：${await responseError(deleted)}`);
    }

    const size = (statSync(file).size / 1024 / 1024).toFixed(2);
    console.log(`[release] 上传 ${name} (${size} MiB) ...`);
    const uploaded = await fetch(`${uploadUrl}?name=${encodeURIComponent(name)}`, {
      method: 'POST',
      headers: { ...authHeaders(token), 'Content-Type': 'application/octet-stream' },
      body: readFileSync(file),
    });
    if (!uploaded.ok) throw new Error(`上传 ${name} 失败：${await responseError(uploaded)}`);
    console.log(`[release] ${name} 上传完成。`);
  }
}

module.exports = {
  ROOT,
  ask,
  ensureToken,
  findOrCreateRelease,
  githubRemote,
  listMsiFiles,
  productName,
  releaseTag,
  run,
  tauriMsiDirectory,
  uploadFiles,
};
