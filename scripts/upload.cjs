const {
  ask,
  ensureToken,
  findOrCreateRelease,
  githubRemote,
  listMsiFiles,
  productName,
  releaseTag,
  uploadFiles,
} = require('./_shared.cjs');

(async () => {
  const files = listMsiFiles();
  if (files.length === 0) throw new Error('未找到 MSI 安装包，请先运行 npm run build。');

  console.log('将上传以下安装包：');
  files.forEach((file) => console.log(`  - ${file}`));

  const tag = releaseTag();
  const remote = githubRemote();
  const token = await ensureToken(remote.owner, remote.repo);
  const notes = await ask('发布说明', `${productName()} ${tag}`);
  const release = await findOrCreateRelease(token, remote.owner, remote.repo, tag, notes);

  await uploadFiles(token, release, files);
  console.log(`\n上传完成：https://github.com/${remote.owner}/${remote.repo}/releases/tag/${tag}`);
})().catch((error) => {
  console.error(`\n上传失败：${error.message}`);
  process.exitCode = 1;
});
