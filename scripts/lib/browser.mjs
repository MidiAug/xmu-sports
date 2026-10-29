import {chromium} from "playwright";

export function launchBrowser(channel = process.env.XMU_TEST_BROWSER ?? "msedge") {
  return chromium.launch({headless:true,channel});
}
