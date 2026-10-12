import { readFileSync, writeFileSync, mkdirSync, readdirSync, realpathSync } from 'node:fs';
import { resolve, relative, isAbsolute, join } from 'node:path';
import { execFileSync } from 'node:child_process';
import { pathToFileURL } from 'node:url';

export function glob(pattern, path) {
  let expression = '';
  for (let i=0;i<pattern.length;i++) {
    if (pattern[i]==='*' && pattern[i+1]==='*') {
      if (pattern[i+2]==='/') {expression+='(?:.*/)?';i+=2;} else {expression+='.*';i++;}
    } else if (pattern[i]==='*') expression+='[^/]*';
    else expression+=pattern[i].replace(/[.+?^${}()|[\]\\]/g,'\\$&');
  }
  return new RegExp(`^${expression}$`).test(path);
}
export const matches = (patterns, path) => patterns.some(pattern => glob(pattern, path));
const number = value => {
  if (!/^\d+$/.test(value)) throw new Error(`Invalid LCOV counter: ${value}`);
  const n = Number(value);
  if (!Number.isSafeInteger(n)) throw new Error('Unsafe LCOV counter');
  return n;
};
export function sourcePath(source, root) {
  const absolute = resolve(root, source);
  const path = relative(root, absolute).replaceAll('\\', '/');
  if (!path || path.startsWith('../') || isAbsolute(path)) throw new Error('Coverage source escapes checkout');
  return path;
}
export function parseLcov(input, root) {
  const records = [];
  let record;
  for (const raw of input.split(/\r?\n/)) {
    const line = raw.trim();
    if (!line) continue;
    if (line.startsWith('SF:')) {
      if (record) throw new Error('Unterminated LCOV record');
      record = { path: sourcePath(line.slice(3), root), lines: {}, found: undefined, hit: undefined };
    } else if (line === 'end_of_record') {
      if (!record || record.found === undefined || record.hit === undefined || record.hit > record.found) throw new Error('Missing or inconsistent LF/LH');
      if (records.some(r => r.path === record.path)) throw new Error('Duplicate LCOV source');
      if (Object.keys(record.lines).length > record.found) throw new Error('DA lines exceed LF');
      records.push(record); record = undefined;
    } else if (line.startsWith('DA:')) {
      if (!record) throw new Error('DA without source');
      const fields = line.slice(3).split(',');
      if (fields.length < 2 || fields.length > 3) throw new Error('Invalid DA record');
      const index = number(fields[0]); const hits = number(fields[1]);
      if (index < 1 || Object.hasOwn(record.lines, index)) throw new Error('Invalid or duplicate DA line');
      record.lines[index] = hits;
    } else if (/^(LF|LH):/.test(line)) {
      if (!record) throw new Error('Counter without source');
      const field = line.startsWith('LF:') ? 'found' : 'hit';
      if (record[field] !== undefined) throw new Error('Duplicate summary counter');
      record[field] = number(line.slice(3));
    } else if (!/^(TN|FN|FNDA|FNF|FNH|BRDA|BRF|BRH):/.test(line)) {
      throw new Error(`Unrecognized LCOV field: ${line.slice(0,20)}`);
    }
  }
  if (record || records.length === 0) throw new Error('Empty or incomplete coverage report');
  return records;
}
export const totals = files => {
  const found = files.reduce((n, f) => n + f.found, 0);
  const hit = files.reduce((n, f) => n + f.hit, 0);
  return { found, hit, percent: found ? 100 * hit / found : null };
};
export function validatePolicy(policy) {
  if (policy.version !== 1 || !policy.label || !Array.isArray(policy.roots) || !policy.roots.length || !Array.isArray(policy.extensions) || !Array.isArray(policy.exclusions) || !Array.isArray(policy.unmeasured) || !Array.isArray(policy.groups)) throw new Error('Invalid coverage policy');
  const thresholds = [policy.floor, policy.target, policy.patchFloor, ...policy.groups.flatMap(g => [g.floor,g.target])];
  if (thresholds.some(n => typeof n !== 'number' || n < 0 || n > 100)) throw new Error('Invalid threshold');
  if (policy.floor > policy.target || policy.groups.some(g => !g.name || !g.paths?.length || g.floor > g.target)) throw new Error('Invalid target or group');
  if (new Set(policy.groups.map(g => g.name)).size !== policy.groups.length) throw new Error('Duplicate group');
  for (const rule of [...policy.exclusions,...policy.unmeasured]) if (!rule.paths?.length || !rule.reason) throw new Error('Undocumented exclusion');
}
export function classify(path, policy) {
  if (!policy.roots.some(root => path.startsWith(`${root}/`)) || !policy.extensions.some(ext => path.endsWith(ext))) return { status:'outside-application' };
  const excluded = policy.exclusions.find(rule => matches(rule.paths,path));
  if (excluded) return { status:'excluded',reason:excluded.reason };
  const unsupported = policy.unmeasured.find(rule => matches(rule.paths,path));
  if (unsupported) return { status:'unmeasured',reason:unsupported.reason };
  return { status:'eligible' };
}
export function parseDiff(text) {
  const changed = new Map(); let path; let next;
  for (const line of text.split('\n')) {
    if (line.startsWith('+++ b/')) { path=line.slice(6); changed.set(path,new Set()); }
    else if (line.startsWith('+++ /dev/null')) path=undefined;
    else if (line.startsWith('@@')) {
      const match=/\+(\d+)(?:,(\d+))? @@/.exec(line); if (!match) throw new Error('Invalid diff hunk'); next=Number(match[1]);
    } else if (path && next !== undefined) {
      if (line.startsWith('+')) {changed.get(path).add(next++);}
      else if (!line.startsWith('-') && !line.startsWith('\\')) next++;
    }
  }
  return changed;
}
export function patchCoverage(records, changed, policy) {
  const files=[]; const missing=[]; const unsupported=[]; let hit=0,found=0;
  for (const [path,lines] of changed) {
    if (!lines.size) continue;
    const type=classify(path,policy);
    if (type.status==='unmeasured') {unsupported.push({path,reason:type.reason});continue;}
    if (type.status!=='eligible') continue;
    const record=records.find(r=>r.path===path);
    if (!record) {missing.push(path);continue;}
    const executable=[...lines].filter(n=>Object.hasOwn(record.lines,n));
    const covered=executable.filter(n=>record.lines[n]>0).length;
    found+=executable.length;hit+=covered;
    files.push({path,found:executable.length,hit:covered,uncovered:executable.filter(n=>record.lines[n]===0)});
  }
  return {found,hit,percent:found?100*hit/found:null,status:missing.length?'failed':found?'measured':'not-applicable',missing,unsupported,files};
}
const below=(total,floor)=>total.found===0 || total.hit*100 < total.found*floor;
export function buildReport(records, inventory, policy, metadata={}, changed) {
  validatePolicy(policy);
  const application=records.filter(r=>classify(r.path,policy).status==='eligible');
  const measured=totals(application);
  if (!measured.found) throw new Error('No measured application lines');
  const groups=policy.groups.map(g=>({name:g.name,floor:g.floor,target:g.target,...totals(application.filter(r=>matches(g.paths,r.path)))}));
  const failures=[];
  if (below(measured,policy.floor)) failures.push(`Application coverage is below ${policy.floor}%`);
  for (const group of groups) if (below(group,group.floor)) failures.push(`${group.name} coverage is below ${group.floor}%`);
  const patch=changed?patchCoverage(records,changed,policy):{status:'not-requested'};
  if (patch.status==='failed' || (patch.status==='measured' && below(patch,policy.patchFloor))) failures.push(`Changed-line coverage fails ${policy.patchFloor}% floor`);
  const missing=inventory.filter(path=>classify(path,policy).status==='eligible' && !records.some(r=>r.path===path));
  const unmeasured=inventory.filter(path=>classify(path,policy).status==='unmeasured').map(path=>({path,reason:classify(path,policy).reason}));
  const excluded=inventory.filter(path=>classify(path,policy).status==='excluded').map(path=>({path,reason:classify(path,policy).reason}));
  return {version:1,label:policy.label,sourceSha:metadata.sourceSha,runUrl:metadata.runUrl,generatedAt:metadata.generatedAt??new Date().toISOString(),status:failures.length?'failed':'passed',raw:totals(records),application:{...measured,floor:policy.floor,target:policy.target},groups,patch,missing,unmeasured,excluded,files:application.map(({lines,...f})=>({...f,percent:f.found?100*f.hit/f.found:null,uncovered:Object.values(lines).filter(h=>h===0).length})),failures};
}
export const escapeHtml=value=>String(value).replace(/[&<>"']/g,c=>({'&':'&amp;','<':'&lt;','>':'&gt;','"':'&quot;',"'":'&#39;'}[c]));
const pct=value=>value===null?'not measured':`${value.toFixed(2)}%`;
export function markdown(report) {
  const safe=value=>String(value).replace(/[|\r\n]/g,' ').replaceAll('`','');
  return `# ${safe(report.label)} coverage\n\nCommit: \`${safe(report.sourceSha??'local')}\` · ${safe(report.generatedAt)}\n\n| Measure | Covered / total | Coverage | Enforced / target |\n|---|---:|---:|---:|\n| Raw collector | ${report.raw.hit}/${report.raw.found} | ${pct(report.raw.percent)} | — |\n| Application | ${report.application.hit}/${report.application.found} | ${pct(report.application.percent)} | ${report.application.floor}% / ${report.application.target}% |\n${report.groups.map(g=>`| ${safe(g.name)} | ${g.hit}/${g.found} | ${pct(g.percent)} | ${g.floor}% / ${g.target}% |`).join('\n')}\n\nPatch: **${report.patch.status}**${report.patch.percent===undefined?'':` (${pct(report.patch.percent)})`}.\n\nResult: **${report.status}**.\n${report.failures.map(f=>`- ${safe(f)}`).join('\n')}\n\n## Largest uncovered areas\n\n${[...report.files].sort((a,b)=>(b.found-b.hit)-(a.found-a.hit)).slice(0,15).map(f=>`- \`${safe(f.path)}\`: ${f.found-f.hit} uncovered collector lines (${pct(f.percent)})`).join('\n')}\n\n## Eligible sources absent from measurement\n\n${report.missing.map(f=>`- \`${safe(f)}\``).join('\n')||'None.'}\n\n## Platform sources not measured\n\n${report.unmeasured.map(f=>`- \`${safe(f.path)}\`: ${safe(f.reason)}`).join('\n')||'None.'}\n\n## Documented exclusions\n\n${report.excluded.map(f=>`- \`${safe(f.path)}\`: ${safe(f.reason)}`).join('\n')||'None.'}\n\nCoverage does not certify native providers, physical devices or store delivery.\n`;
}
export function badge(report) {
  const value=`${report.application.percent.toFixed(1)}%`;
  const color=report.status!=='passed'?'#b91c1c':report.application.percent>=report.application.target?'#15803d':'#a16207';
  const label=escapeHtml(report.label);const width=Math.max(110,report.label.length*7+12);
  return `<svg xmlns="http://www.w3.org/2000/svg" width="${width+64}" height="20" role="img" aria-label="${label}: ${value}"><title>${label}: ${value}</title><rect width="${width}" height="20" fill="#334155"/><rect x="${width}" width="64" height="20" fill="${color}"/><g fill="white" font-family="Verdana,sans-serif" font-size="11" text-anchor="middle"><text x="${width/2}" y="14">${label}</text><text x="${width+32}" y="14">${value}</text></g></svg>\n`;
}
export function html(report, records, root) {
  const header=`<!doctype html><html lang="en"><meta charset="utf-8"><meta name="viewport" content="width=device-width"><title>${escapeHtml(report.label)} coverage</title><style>body{font:15px system-ui;margin:2rem;color:#172033}pre{overflow:auto;line-height:1.6}.miss{background:#fee2e2}.hit{background:#dcfce7}details{margin:1rem 0}summary{cursor:pointer}a{color:#4338ca}.line{display:block}code{font:13px monospace}</style><h1>${escapeHtml(report.label)} coverage</h1><p>${escapeHtml(report.sourceSha??'local')} · ${escapeHtml(report.generatedAt)}</p><p>Raw ${pct(report.raw.percent)}; application ${pct(report.application.percent)}. Status: ${report.status}.</p>`;
  let body='';
  for (const record of records.filter(r=>report.files.some(f=>f.path===r.path))) {
    const path=resolve(root,record.path);
    if (sourcePath(realpathSync(path),realpathSync(root))!==record.path) throw new Error('Source symlink escapes inventory');
    const source=readFileSync(path,'utf8').split('\n');
    if (Object.keys(record.lines).some(n=>Number(n)>source.length)) throw new Error('Coverage references nonexistent source line');
    body+=`<details><summary>${escapeHtml(record.path)} — ${pct(record.found?100*record.hit/record.found:null)}</summary><pre><code>${source.map((line,i)=>`<span class="line ${record.lines[i+1]===0?'miss':record.lines[i+1]>0?'hit':''}">${i+1}: ${escapeHtml(line)}</span>`).join('')}</code></pre></details>`;
  }
  return `${header}<h2>Source lines</h2>${body}<h2>Measurement limitations</h2><pre>${escapeHtml(markdown(report))}</pre></html>`;
}
export function inventory(root,policy) {
  const files=[];
  function visit(dir) {
    for (const entry of readdirSync(dir,{withFileTypes:true})) {
      const file=join(dir,entry.name);
      if (entry.isDirectory()) visit(file);
      else if (entry.isFile() && policy.extensions.some(ext=>file.endsWith(ext))) files.push(relative(root,file).replaceAll('\\','/'));
    }
  }
  for (const dir of policy.roots) visit(resolve(root,dir));
  return files.sort();
}
export function cli(args) {
  const options=new Map();
  for (let i=0;i<args.length;i+=2) {if (!args[i].startsWith('--')||args[i+1]===undefined) throw new Error('Use --option value');options.set(args[i].slice(2),args[i+1]);}
  const root=resolve(options.get('root')??'.');
  const policy=JSON.parse(readFileSync(resolve(root,options.get('policy')??'tool/coverage-policy.json'),'utf8'));
  const input=readFileSync(resolve(root,options.get('input')??'coverage/lcov.info'),'utf8');
  const records=parseLcov(input,root);let changed;
  if (options.has('base')) {
    const base=options.get('base');if (!/^[a-f0-9]{40}$/.test(base)) throw new Error('Use a full PR base SHA');
    const diff=execFileSync('git',['diff','--no-ext-diff','--no-renames','--unified=0',base,'--',...policy.roots],{cwd:root,encoding:'utf8'});changed=parseDiff(diff);
  }
  const sourceSha=process.env.GITHUB_SHA??execFileSync('git',['rev-parse','HEAD'],{cwd:root,encoding:'utf8'}).trim();
  const runUrl=process.env.GITHUB_RUN_ID?`https://github.com/${process.env.GITHUB_REPOSITORY}/actions/runs/${process.env.GITHUB_RUN_ID}`:undefined;
  const report=buildReport(records,inventory(root,policy),policy,{sourceSha,runUrl},changed);
  const output=resolve(root,options.get('output')??'coverage/report');mkdirSync(output,{recursive:true});
  writeFileSync(join(output,'summary.json'),JSON.stringify(report,null,2)+'\n');
  writeFileSync(join(output,'README.md'),markdown(report));writeFileSync(join(output,'badge.svg'),badge(report));
  writeFileSync(join(output,'index.html'),html(report,records,root));
  const normalized=input.replace(/^SF:(.+)$/gm,(_,sf)=>`SF:${sourcePath(sf,root)}`);writeFileSync(join(output,'lcov.info'),normalized);
  if (process.env.GITHUB_STEP_SUMMARY) writeFileSync(process.env.GITHUB_STEP_SUMMARY,markdown(report),{flag:'a'});
  console.log(`${report.label}: raw ${pct(report.raw.percent)}, application ${pct(report.application.percent)}; ${report.status}`);
  for (const group of report.groups) console.log(`${group.name}: ${pct(group.percent)} (${group.hit}/${group.found}); floor ${group.floor}%`);
  if (report.failures.length) {for (const failure of report.failures) console.error(failure);process.exitCode=1;}
  return report;
}
if (process.argv[1] && import.meta.url===pathToFileURL(resolve(process.argv[1])).href) {
  try {cli(process.argv.slice(2));} catch (error) {console.error(error.message);process.exitCode=1;}
}
