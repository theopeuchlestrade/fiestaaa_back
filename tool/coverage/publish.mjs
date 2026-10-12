import { readFileSync } from 'node:fs';
import { resolve } from 'node:path';
import { pathToFileURL } from 'node:url';
import { badge, markdown } from './report.mjs';

export function publicationAllowed(env, report, mainSha) {
  return env.SOURCE_EVENT === 'push' && env.SOURCE_REF === 'refs/heads/main' && env.SOURCE_CI_RESULT === 'success' && /^[a-f0-9]{40}$/.test(env.SOURCE_SHA ?? '') && report.sourceSha === env.SOURCE_SHA && report.status === 'passed' && env.SOURCE_REPOSITORY === env.GITHUB_REPOSITORY && mainSha === env.SOURCE_SHA;
}
export function validatePublishedReport(report) {
  if (report.version !== 1 || !/^[a-f0-9]{40}$/.test(report.sourceSha ?? '') || report.status !== 'passed' || !report.label || !report.application?.found || !Array.isArray(report.groups) || !Array.isArray(report.files) || !report.groups.length || !report.raw?.found || !Array.isArray(report.failures) || report.failures.length) throw new Error('Publication requires a validated successful report');
  for (const total of [report.raw,report.application,...report.groups]) {
    if (!Number.isSafeInteger(total.hit) || !Number.isSafeInteger(total.found) || total.hit < 0 || total.hit > total.found || total.found <= 0 || !Number.isFinite(total.percent) || Math.abs(total.percent - 100*total.hit/total.found)>1e-8) throw new Error('Inconsistent publication totals');
    if (total.floor !== undefined && total.percent < total.floor) throw new Error('Publication report misses threshold');
  }
}
export async function publish(env, report, api) {
  validatePublishedReport(report);
  if (!publicationAllowed(env,report,env.SOURCE_SHA)) return {published:false,reason:'Only successful main push CI can publish'};
  if (!/^\d+$/.test(env.SOURCE_RUN_ID ?? '')) throw new Error('Invalid source CI run');
  const run=await api('GET',`actions/runs/${env.SOURCE_RUN_ID}`);
  if (run.status !== 'completed' || run.conclusion !== 'success' || run.event !== 'push' || run.head_branch !== 'main' || run.head_sha !== report.sourceSha || run.head_repository?.full_name !== env.GITHUB_REPOSITORY) return {published:false,reason:'Source run is not trusted successful main CI'};
  const main=await api('GET','git/ref/heads/main');
  if (!publicationAllowed(env,report,main.object.sha)) return {published:false,reason:'Main superseded this run'};
  let previous;
  try {previous=await api('GET','git/ref/heads/coverage-reports');} catch (error) {if (error.status!==404) throw error;}
  let baseTree;
  if (previous) baseTree=(await api('GET',`git/commits/${previous.object.sha}`)).tree.sha;
  const tree=await api('POST','git/trees',{
    ...(baseTree?{base_tree:baseTree}:{}),
    tree:[
      {path:'badge.svg',mode:'100644',type:'blob',content:badge(report)},
      {path:'summary.json',mode:'100644',type:'blob',content:JSON.stringify(report)+'\n'},
      {path:'README.md',mode:'100644',type:'blob',content:markdown(report)+`\n[Source CI](${report.runUrl})\n`},
    ],
  });
  const commit=await api('POST','git/commits',{message:`chore: record coverage for ${report.sourceSha.slice(0,7)}`,tree:tree.sha,parents:previous?[previous.object.sha]:[]});
  // Recheck immediately before updating the public ref. The workflow serializes
  // publishers; a non-fast-forward ref update fails rather than overwriting.
  const current=await api('GET','git/ref/heads/main');
  if (current.object.sha!==report.sourceSha) return {published:false,reason:'Main advanced before publication'};
  if (previous) await api('PATCH','git/refs/heads/coverage-reports',{sha:commit.sha,force:false});
  else await api('POST','git/refs',{ref:'refs/heads/coverage-reports',sha:commit.sha});
  return {published:true,sha:commit.sha};
}
export async function cli(args) {
  const file=args[0]??'coverage/report/summary.json';
  const report=JSON.parse(readFileSync(file,'utf8'));
  const env=process.env;
  if (!/^[\w.-]+\/[\w.-]+$/.test(env.GITHUB_REPOSITORY??'') || !env.GITHUB_TOKEN) throw new Error('Publication requires repository and scoped GITHUB_TOKEN');
  const api=async(method,path,body)=>{
    const response=await fetch(`https://api.github.com/repos/${env.GITHUB_REPOSITORY}/${path}`,{method,headers:{Authorization:`Bearer ${env.GITHUB_TOKEN}`,Accept:'application/vnd.github+json','X-GitHub-Api-Version':'2022-11-28',...(body?{'Content-Type':'application/json'}:{})},...(body?{body:JSON.stringify(body)}:{})});
    if (!response.ok) {const error=new Error(`GitHub publication ${method} ${path}: HTTP ${response.status}`);error.status=response.status;throw error;}
    return response.json();
  };
  const result=await publish(env,report,api);console.log(JSON.stringify(result));return result;
}
if (process.argv[1] && import.meta.url===pathToFileURL(resolve(process.argv[1])).href) {
  try {await cli(process.argv.slice(2));} catch(error) {console.error(error.message);process.exitCode=1;}
}
