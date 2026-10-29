import badminton from "../assets/icons/badminton-line.svg?raw";
import pingpong from "../assets/icons/pingpong-line.svg?raw";
import gym from "../assets/icons/barbell-line.svg?raw";
import swim from "../assets/icons/swimming-line.svg?raw";
import bike from "../assets/icons/bike-line.svg?raw";
import yoga from "../assets/icons/yoga-line.svg?raw";
import run from "../assets/icons/run-line.svg?raw";
export { default as locationIcon } from "../assets/icons/location-line.svg?raw";
export { default as calendarIcon } from "../assets/icons/calendar-line.svg?raw";

export function sportIcon(value: string): string {
  if (/羽毛/.test(value)) return badminton;
  if (/乒乓|匹克|网球/.test(value)) return pingpong;
  if (/健身/.test(value)) return gym;
  if (/游泳/.test(value)) return swim;
  if (/单车/.test(value)) return bike;
  if (/瑜伽/.test(value)) return yoga;
  return run;
}
