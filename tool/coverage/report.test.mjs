import test from 'node:test';
import assert from 'node:assert/strict';
import { mkdtempSync,mkdirSync,writeFileSync,rmSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import {parseLcov,buildReport,patchCoverage,parseDiff,badge,markdown,html,glob,sourcePath,validatePolicy} from './report.mjs';
import {publish,publicationAllowed} from './publish.mjs';
const root='/fixture';
const sha='a'.repeat(40);
const policy={version:1,label:'Dart VM lines',roots:['lib'],extensions:['.dart'],floor:50,target:70,patchFloor:80,exclusions:[{paths:['lib/l10n/**'],reason:'Generated'}],unmeasured:[{paths:['lib/**/*_web.dart'],reason:'Browser adapter'}],groups:[{name:'critical',paths:['lib/a.dart'],floor:50,target:80}]};
const input='SF:lib/a.dart\nDA:1,2\nDA:2,0\nLF:2\nLH:1\nend_of_record\n';
const record=parseLcov(input,root);
function report(){return buildReport(record,['lib/a.dart'],policy,{sourceSha:sha,generatedAt:'2026-01-01',runUrl:'https://github.com/example/app/actions/runs/1'});}
test('LCOV counters and source paths are validated',()=>{
 assert.equal(record[0].path,'lib/a.dart');assert.equal(record[0].lines[1],2);assert.equal(record[0].hit,1);
 assert.throws(()=>sourcePath('../secret',root),/escapes/);
 assert.throws(()=>sourcePath('/other/lib/a.dart',root),/escapes/);
});
for(const [name,value] of [['empty',''],['incomplete',input.replace('end_of_record','')],['missing counter',input.replace('LF:2\n','')],['negative',input.replace('DA:1,2','DA:1,-2')],['unsafe integer',input.replace('LF:2','LF:99999999999999999')],['invalid summary',input.replace('LH:1','LH:3')],['duplicate source',input+input],['duplicate line',input.replace('DA:2,0','DA:1,0')],['unknown field',input.replace('DA:2,0','BROKEN:2')]]) test(`rejects ${name} report`,()=>assert.throws(()=>parseLcov(value,root)));
test('LLVM summary accounting is not replaced by unique DA counts',()=>{
 const records=parseLcov(input.replace('LF:2','LF:3').replace('LH:1','LH:2'),root);
 const result=buildReport(records,['lib/a.dart'],policy);
 assert.equal(result.application.percent,100*2/3);assert.equal(result.application.found,3);
});
test('global counters are weighted and generated exclusions are explicit',()=>{
 const records=parseLcov(input+'SF:lib/l10n/en.dart\nDA:1,0\nLF:10\nLH:0\nend_of_record\n',root);
 const result=buildReport(records,['lib/a.dart','lib/missing.dart','lib/l10n/en.dart','lib/x_web.dart'],policy);
 assert.equal(result.raw.found,12);assert.equal(result.application.percent,50);
 assert.deepEqual(result.missing,['lib/missing.dart']);assert.equal(result.unmeasured[0].path,'lib/x_web.dart');assert.equal(result.excluded.length,1);
});
test('zero measured application fails instead of a green zero-line badge',()=>assert.throws(()=>buildReport(parseLcov(input.replaceAll('lib/a.dart','lib/l10n/en.dart'),root),[],policy),/No measured/));
test('threshold equality passes and one fraction below fails',()=>{
 assert.equal(report().status,'passed');
 assert.equal(buildReport(record,[],{...policy,floor:50.01}).status,'failed');
 assert.equal(buildReport(record,[],{...policy,groups:[{...policy.groups[0],floor:50.01}]}).status,'failed');
});
test('absent critical group fails even with passing global coverage',()=>assert.equal(buildReport(record,[],{...policy,groups:[{name:'missing',paths:['lib/none.dart'],floor:0,target:80}]}).status,'failed'));
test('invalid policies reject undocumented exclusions and inverted floors',()=>{
 assert.throws(()=>validatePolicy({...policy,exclusions:[{paths:['lib/a.dart']}]}));assert.throws(()=>validatePolicy({...policy,floor:90}));
});
test('glob distinguishes path stars from recursive stars',()=>{
 assert.equal(glob('lib/*.dart','lib/a.dart'),true);assert.equal(glob('lib/*.dart','lib/src/a.dart'),false);assert.equal(glob('lib/**/*.dart','lib/src/a.dart'),true);
});
test('patch parses added line ranges without counting removed lines',()=>{
 const diff='diff --git a/lib/a.dart b/lib/a.dart\n--- a/lib/a.dart\n+++ b/lib/a.dart\n@@ -1,2 +1,2 @@\n-old\n+new\n same\n';
 const changed=parseDiff(diff);assert.deepEqual([...changed.get('lib/a.dart')],[1]);
 assert.equal(patchCoverage(record,changed,policy).percent,100);
 const rejected=buildReport(record,[],policy,{},new Map([['lib/a.dart',new Set([1,2])]]));assert.equal(rejected.status,'failed');
});
test('missing eligible file fails patch; documentation and browser-only are explicit',()=>{
 assert.equal(patchCoverage(record,new Map([['lib/new.dart',new Set([1])]]),policy).status,'failed');
 assert.equal(patchCoverage(record,new Map([['README.md',new Set([1])]]),policy).status,'not-applicable');
 assert.equal(patchCoverage(record,new Map([['lib/src/a_web.dart',new Set([1])]]),policy).unsupported.length,1);
 assert.equal(patchCoverage(record,new Map([['lib/a.dart',new Set([20])]]),policy).status,'not-applicable');
});
test('SVG and HTML escape source content and preserve correct commit',()=>{
 const result=report();result.label='<script>&';assert(!badge(result).includes('<script>'));assert(markdown(result).includes(sha));
 const dir=mkdtempSync(join(tmpdir(),'fiestaaa-coverage-test-'));
 try {mkdirSync(join(dir,'lib'));writeFileSync(join(dir,'lib/a.dart'),'<script>one</script>\ntwo\n');const page=html(result,record,dir);assert(page.includes('&lt;script&gt;'));assert(page.includes('class="line miss"'));assert(!page.includes('<script>'));} finally {rmSync(dir,{recursive:true,force:true});}
});
const env={SOURCE_EVENT:'push',SOURCE_REF:'refs/heads/main',SOURCE_CI_RESULT:'success',SOURCE_SHA:sha,SOURCE_RUN_ID:'1',SOURCE_REPOSITORY:'example/app',GITHUB_REPOSITORY:'example/app'};
function fakeApi({advanced=false,existing=false}={}) {
 const calls=[];let reads=0;
 const api=async(method,path,body)=>{
  calls.push({method,path,body});
  if(path==='actions/runs/1')return {status:'completed',conclusion:'success',event:'push',head_branch:'main',head_sha:sha,head_repository:{full_name:'example/app'}};
  if(path==='git/ref/heads/main') return {object:{sha:advanced&&reads++>0?'b'.repeat(40):sha}};
  if(path==='git/ref/heads/coverage-reports') {if(existing)return {object:{sha:'previous'}};const e=new Error();e.status=404;throw e;}
  if(path==='git/commits/previous')return {tree:{sha:'tree'}};
  return {sha:'created'};
 };return {api,calls};
}
test('publication permits only successful trusted main with matching SHA',()=>{
 assert(publicationAllowed(env,report(),sha));
 for(const change of [{SOURCE_EVENT:'pull_request'},{SOURCE_REF:'refs/heads/fork'},{SOURCE_CI_RESULT:'failure'},{SOURCE_SHA:'b'.repeat(40)}])assert(!publicationAllowed({...env,...change},report(),sha));
 assert(!publicationAllowed(env,report(),'b'.repeat(40)));
});
test('fork and failed CI do not call write API',async()=>{
 const {api,calls}=fakeApi();assert.equal((await publish({...env,SOURCE_EVENT:'pull_request'},report(),api)).published,false);assert.equal(calls.length,0);
 assert.equal((await publish({...env,SOURCE_CI_RESULT:'failure'},report(),api)).published,false);assert.equal(calls.length,0);
});
test('main publisher creates only compact files and an orphan badge branch',async()=>{
 const {api,calls}=fakeApi();assert.equal((await publish(env,report(),api)).published,true);
 const tree=calls.find(c=>c.path==='git/trees').body.tree;assert.deepEqual(tree.map(f=>f.path),['badge.svg','summary.json','README.md']);
 assert.deepEqual(calls.find(c=>c.path==='git/commits').body.parents,[]);assert.equal(calls.at(-1).path,'git/refs');
});
test('existing branch update never force pushes',async()=>{
 const {api,calls}=fakeApi({existing:true});await publish(env,report(),api);assert.equal(calls.at(-1).body.force,false);assert.deepEqual(calls.find(c=>c.path==='git/commits').body.parents,['previous']);
});
test('main advances before final ref update: no badge publication',async()=>{
 const {api,calls}=fakeApi({advanced:true});assert.equal((await publish(env,report(),api)).published,false);assert(!calls.some(c=>c.method==='PATCH'||c.path==='git/refs'));
});
test('malformed publication totals and failed reports fail closed',async()=>{
 const {api,calls}=fakeApi();const bad=report();bad.application.percent=100;
 await assert.rejects(publish(env,bad,api));await assert.rejects(publish(env,{...report(),status:'failed'},api));assert.equal(calls.length,0);
});
