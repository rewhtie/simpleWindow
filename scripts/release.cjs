const {
  ROOT,
  ask,
  ensureToken,
  findOrCreateRelease,
  githubRemote,
  listMsiFiles,
  productName,
  releaseTag,
  run,
  uploadFiles,
} = require('./_shared.cjs');

(async () => {
  const tag = releaseTag();
  console.log(`==> 步骤 1/3：构建 ${productName()} ${tag} MSI\n`);
  await run('npm', ['run', 'build'], { cwd: ROOT });

  const files = listMsiFiles();
  if (files.length === 0) throw new Error('构建完成，但未找到 MSI 安装包。');
  console.log('\n构建产物：');
  files.forEach((file) => console.log(`  - ${file}`));

  const shouldUpload = await ask('\n是否上传到 GitHub Releases？', 'y');
  if (!/^y(?:es)?$/i.test(shouldUpload)) {
    console.log('已跳过上传。');
    return;
  }
 
  const remote = githubRemote();
  const token = await ensureToken(remote.owner, remote.repo);
  const notes = await ask('发布说明', `${productName()} ${tag}`);

  console.log(`\n==> 步骤 2/3：创建或查找 ${remote.owner}/${remote.repo} 的 ${tag}\n`);
  const release = await findOrCreateRelease(token, remote.owner, remote.repo, tag, notes);

  console.log('\n==> 步骤 3/3：上传 MSI\n');
  await uploadFiles(token, release, files);
  console.log(`\n发布完成：https://github.com/${remote.owner}/${remote.repo}/releases/tag/${tag}`);
})().catch((error) => {
  console.error(`\n发布失败：${error.message}`);
  process.exitCode = 1;
});
