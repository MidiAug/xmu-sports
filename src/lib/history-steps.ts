import type { RunEvent } from "../types";
import { progressMessage, progressStep } from "./booking-progress";
import { formatStepLine } from "./format-human";
import type { StepCycleBlock } from "./collapse-step-cycles";

function stageKey(event:RunEvent):string {
  if (["spawn","done","fail","cancelled"].includes(event.action ?? "")) return event.action!;
  const phase=event.http?.phase ?? event.result ?? "";
  if (phase==="scheduled_waiting_fire") return "waiting";
  if (phase.startsWith("worker_") || phase==="scheduled_h5_ready") return "prepare";
  if (phase==="checking_orders") return "dedupe";
  if (["waiting_order_sync","rechecking_orders","order_check_retry","order_reconciled","verifying_order"].includes(phase)) return "order";
  return ({auth:"auth",query:"query",captcha:"captcha",verify:"verify",submit:"submit",order:"order"} as Record<string,string>)[progressStep(event)] ?? "other";
}

export function historySteps(events: RunEvent[]): Array<StepCycleBlock & { elapsedMs: number; stage:string }> {
  const rows: Array<StepCycleBlock & { elapsedMs: number; key: string; stage:string }> = [];
  const outcome=[...events].reverse().find(event=>event.action==="done")?.result;
  for (const event of events) {
    if (event.action === "http_exchange") continue;
    const semantic = ["http_phase","auth_check","login_recovery","spawn","done","fail","cancelled"].includes(event.action ?? "");
    let line = semantic ? progressMessage(event) : formatStepLine(event);
    if (event.action==="spawn" && outcome==="http_auth_ready") line="开始检查体育登录，不提交预约";
    if (event.action==="spawn" && outcome==="http_prepared") line="开始检查预约条件，不提交订单";
    if (line === "步骤") line = `状态记录：${event.result ?? event.action ?? "未提供说明"}`;
    if (event.action === "fail" && event.http?.phase) {
      const phase = progressMessage({...event, action:"http_phase", result:event.http.phase, http:{phase:event.http.phase}});
      line = `${phase}时停止：${event.http.message ?? event.result ?? "未返回原因"}`;
    }
    const key = JSON.stringify([event.action,event.http?.phase ?? event.result,line]);
    const previous = rows.at(-1);
    if (previous?.key === key) { previous.k++; previous.tsLast=event.ts; }
    else rows.push({key,stage:stageKey(event),lines:[{line,ts:event.ts}],k:1,tsFirst:event.ts,tsLast:event.ts,elapsedMs:0});
  }
  return rows.map((row,index)=>({...row,elapsedMs:Math.max(0,Date.parse(rows[index+1]?.tsFirst ?? row.tsLast)-Date.parse(row.tsFirst))}));
}

export function historyTarget(events: RunEvent[]): string {
  for (const event of [...events].reverse()) {
    const target=event.http?.submitted_target ?? event.http?.target;
    if (target?.venue) return [target.sport,target.venue,target.date,`${target.time ?? ""}–${target.end ?? ""}`,target.court].filter(Boolean).join("　");
  }
  return "";
}

export function historyStages(events:RunEvent[]) {
  const labels:Record<string,string>={spawn:"开始任务",auth:"验证登录与账号",prepare:"提前准备",waiting:"等待执行时间",dedupe:"检查已有预约",query:"查询实时场次",captcha:"识别验证码",verify:"校验预约条件",submit:"发送预约请求",order:"查找匹配订单"};
  const stages:Array<{key:string;label:string;tsFirst:string;tsLast:string;elapsedMs:number;steps:ReturnType<typeof historySteps>}>=[];
  for (const step of historySteps(events)) {
    const previous=stages.at(-1);
    if (previous?.key===step.stage && !["done","fail","cancelled"].includes(step.stage)) {
      previous.steps.push(step);previous.tsLast=step.tsLast;
    } else stages.push({key:step.stage,label:labels[step.stage] ?? step.lines[0].line,tsFirst:step.tsFirst,tsLast:step.tsLast,elapsedMs:0,steps:[step]});
  }
  const waiting=stages.findIndex(stage=>stage.key==="waiting");
  if (waiting>0 && stages.slice(0,waiting).every(stage=>["spawn","auth","prepare"].includes(stage.key))) {
    const before=stages.splice(0,waiting);
    stages.unshift({key:"prepare",label:"提前准备",tsFirst:before[0].tsFirst,tsLast:before.at(-1)!.tsLast,elapsedMs:0,steps:before.flatMap(stage=>stage.steps)});
    for (const stage of stages.slice(2)) if (stage.key==="auth") stage.label="执行前校验";
  } else if (stages[0]?.key==="spawn" && stages[1]?.key==="auth") {
    const start=stages.shift()!;stages[0].tsFirst=start.tsFirst;stages[0].steps.unshift(...start.steps);
  }
  return stages.map((stage,index)=>({...stage,elapsedMs:Math.max(0,Date.parse(stages[index+1]?.tsFirst ?? stage.tsLast)-Date.parse(stage.tsFirst))}));
}

export function historyTargetParts(events:RunEvent[]) {
  for (const event of [...events].reverse()) {
    const target=event.http?.submitted_target ?? event.http?.target;
    if (target?.venue) return {sport:target.sport ?? "",venue:target.venue,date:target.date ?? "",time:`${target.time ?? ""}–${target.end ?? ""}`,court:target.court ?? ""};
  }
  return null;
}
