// Only a successful test job for this exact commit may replace release tests.
export function latestBranchRun(runs, sha, branch) {
  return runs.filter(run => run.head_sha === sha && run.head_branch === branch && run.event === "push")
    .sort((a, b) => Number(b.id) - Number(a.id))[0];
}

export function testDecision(run, jobs) {
  if (!run) return "test";
  const test = jobs.find(job => job.name === "Test Windows x64");
  if (test?.conclusion === "success") return "reuse";
  if (test && ["failure", "timed_out", "action_required"].includes(test.conclusion)) return "fail";
  if (run.status === "completed" || ["cancelled", "skipped"].includes(test?.conclusion)) return "test";
  return "wait";
}

export async function checkReleaseTests({ github, context, core, sleep = ms => new Promise(resolve => setTimeout(resolve, ms)), attempts = 40 }) {
  const { owner, repo } = context.repo;
  const branch = context.payload.repository.default_branch;
  // Resolve HEAD from checkout (also works for annotated tags).
  const sha = process.env.RELEASE_COMMIT;
  if (!/^[a-f0-9]{40}$/.test(sha ?? "")) throw new Error("Missing release commit SHA.");
  for (let attempt = 0; attempt < attempts; attempt++) {
    const { data } = await github.rest.actions.listWorkflowRuns({ owner, repo,
      workflow_id: "app-desktop-release.yml", head_sha: sha, branch, event: "push", per_page: 100 });
    const run = latestBranchRun(data.workflow_runs, sha, branch);
    const jobs = run ? await github.paginate(github.rest.actions.listJobsForWorkflowRun,
      { owner, repo, run_id: run.id, filter: "latest", per_page: 100 }) : [];
    const decision = testDecision(run, jobs);
    if (decision === "fail") throw new Error(`Tests failed for ${sha}: ${run.html_url}. Fix or rerun CI before releasing.`);
    if (decision === "reuse") {
      const preparation = jobs.find(job => job.name === "Prepare changed release dependencies");
      // Branch and tag are often pushed together. Allow the branch job to save
      // its caches before starting a second expensive cold build on the tag.
      if (preparation && preparation.status !== "completed" && attempt + 1 < attempts) {
        core.info("Tests passed; waiting briefly for default-branch release caches.");
        await sleep(30_000);
        continue;
      }
      core.setOutput("reuse-tests", "true");
      await core.summary.addRaw(`Reusing successful Windows tests for commit ${sha}: ${run.html_url}\n`).write();
      return;
    }
    if (decision === "test") break;
    core.info(`Waiting for tests on the tagged commit (${attempt + 1}/${attempts}).`);
    if (attempt + 1 < attempts) await sleep(30_000);
  }
  core.setOutput("reuse-tests", "false");
  await core.summary.addRaw(`No completed passing tests for ${sha}; running the release test job.\n`).write();
}
