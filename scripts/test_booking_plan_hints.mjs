import assert from 'node:assert/strict';
import {readFileSync} from 'node:fs';
import ts from 'typescript';
const source=readFileSync(new URL('../src/lib/booking-plan-hints.ts',import.meta.url),'utf8');
const js=ts.transpileModule(source,{compilerOptions:{module:ts.ModuleKind.ESNext}}).outputText;
const {hasBookingPlan}=await import(`data:text/javascript;base64,${Buffer.from(js).toString('base64')}`);
const view={plans:{paused:false,rules:[{sport:'gym',account:'a',enabled:true,weekdays:[1,3,5],first_date:'2026-10-30'}],attempts:[]}};
assert.equal(hasBookingPlan(view,'gym','2026-10-30','a'),true);
assert.equal(hasBookingPlan(view,'gym','2026-10-31','a'),false);
assert.equal(hasBookingPlan(view,'gym','2026-10-28','a'),false);
assert.equal(hasBookingPlan(view,'gym','2026-10-30','b'),false);
assert.equal(hasBookingPlan(view,'gym','2026-10-30'),false);
assert.equal(hasBookingPlan(view,'swim','2026-10-30','a'),false);
view.plans.paused=true;assert.equal(hasBookingPlan(view,'gym','2026-10-30','a'),false);
view.plans.paused=false;view.plans.rules[0].enabled=false;
assert.equal(hasBookingPlan(view,'gym','2026-10-30','a'),false);
view.plans.rules[0].enabled=true;
for(const status of ['skipped','cancelled','failed','missed','queued','running','succeeded','existing','unconfirmed']) {
  view.plans.attempts=[{sport:'gym',account:'a',status,task:{target_date:'2026-10-30'}}];
  assert.equal(hasBookingPlan(view,'gym','2026-10-30','a'),['queued','running','succeeded','existing','unconfirmed'].includes(status),status);
}
console.log('Plan hints passed: actual weekdays, first date, verified account, pause, disabled and skipped/executed targets');
