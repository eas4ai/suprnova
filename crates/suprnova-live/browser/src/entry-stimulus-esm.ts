import { installStimulusAdapter } from "./features/stimulus.js";

export const stimulusRegistration = installStimulusAdapter(globalThis);
export { installStimulusAdapter };

export default stimulusRegistration;
