import { sharedValue } from "./constants";
import { Calculator } from "./math";

export function run(): number {
  const calculator = new Calculator();
  return calculator.calculate({ value: sharedValue });
}
