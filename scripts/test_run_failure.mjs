import assert from 'node:assert/strict';
import {readFileSync} from 'node:fs';
import ts from 'typescript';
const compile=source=>'data:text/javascript;base64,'+Buffer.from(ts.transpileModule(source,{compilerOptions:{module:ts.ModuleKind.ESNext,target:ts.ScriptTarget.ES2022}}).outputText).toString('base64');
const failureUrl=compile(readFileSync('src/lib/run-failure.ts','utf8'));
const {diagnoseRunFailure}=await import(failureUrl);
const {outcomeFromEvent}=await import(compile(readFileSync('src/lib/run-outcome.ts','utf8').replace('"./run-failure"',JSON.stringify(failureUrl))));
const base={task_id:'task',run_id:'run',ts:'2026-10-30T15:52:42+08:00'};
const fail={...base,action:'fail',result:'HTTP_FAILED',http:{phase:'checking_orders',captcha_requested:false}};
const exchange=http=>({...base,action:'http_exchange',http:{api:'order_list',method:'POST',phase:'checking_orders',elapsed_ms:1875,request:{p:'2'},...http}});
for(const [error,type,category] of [['timeout',undefined,'请求超时'],['timeout','connect_timeout','连接超时'],['timeout','read_timeout','读取超时'],['connection','proxy','代理连接异常'],['connection','tls','安全连接异常'],['connection',undefined,'网络连接异常'],['invalid_json',undefined,'响应格式异常']]) {
 const row=exchange({error,error_type:type});
 const value=diagnoseRunFailure(fail,[row,fail]);
 assert.equal(value.category,category);assert.equal(value.submission,'未发送预约提交');
 assert.equal(value.api,'查询预约列表');assert.deepEqual(value.request,{p:'2'});
 assert.notEqual(outcomeFromEvent(fail,[row,fail]).title,'学校接口请求失败');
}
const old=diagnoseRunFailure(fail,[exchange({error:'timeout'}),fail]);
assert.match(old.explanation,/未区分连接和读取/);
assert.match(old.explanation,/没有明确限流或参数错误证据/);
assert.equal(outcomeFromEvent(fail,[exchange({error:'timeout'}),fail]).title,'订单查询超时');
assert.equal(outcomeFromEvent({...fail,http:{...fail.http,failure_exchange:exchange({error:'timeout'}).http}},[fail]).title,'订单查询超时','native compact evidence works without raw exchange events');
for(const [status,category] of [[429,'明确限流'],[503,'学校服务异常'],[400,'HTTP请求被拒绝']]) {
 assert.equal(diagnoseRunFailure(fail,[exchange({error:'http_error',http_status:status}),fail]).category,category);
}
assert.equal(diagnoseRunFailure({...fail,result:'API_REJECTED'},[exchange({http_status:200,response:{status:0,info:'参数错误'}})]).category,'学校业务拒绝');
assert.equal(diagnoseRunFailure({...fail,result:'ACCOUNT_LIMIT'},[]).category,'预约条件不满足');
assert.equal(diagnoseRunFailure({...fail,result:'SESSION_EXPIRED'},[]).category,'登录或账号异常');
assert.equal(diagnoseRunFailure({...fail,result:'HTTP_WORKER_ERROR'},[]).category,'本地组件异常');
const sent={...base,action:'http_phase',http:{phase:'submitting'}};
const unknown=outcomeFromEvent({...fail,result:'SUBMIT_UNKNOWN'},[sent,exchange({error:'timeout'})]);
assert.equal(unknown.kind,'info');assert.equal(unknown.failure.category,'结果待确认');
assert.match(unknown.failure.submission,/已进入提交/);assert.doesNotMatch(unknown.detail,/未提交预约/);
assert.doesNotMatch(diagnoseRunFailure(fail,[sent,exchange({error:'timeout'})]).summary,/本次未提交/);
assert.equal(diagnoseRunFailure(fail,[{...exchange({error:'timeout'}),run_id:'other'}]).category,'接口请求失败');
assert.equal(diagnoseRunFailure(fail,[{...exchange({error:'timeout'}),task_id:'other'}]).api,undefined);
const noProof={...fail,http:{phase:'checking_orders'}};
assert.match(diagnoseRunFailure(noProof,[]).submission,/不足以确认/);
assert.doesNotMatch(diagnoseRunFailure(noProof,[exchange({error:'timeout'})]).summary,/本次未提交/);
console.log('Failure diagnostics passed: transport subtypes, legacy uncertainty, explicit rate limits, server/status/format/business categories, run isolation and submission evidence');
