import { registerRuntimeFeature } from "./features/producer.js";
import { uploadsFeature } from "./uploads/feature.js";

export { uploadsFeature };
export { configureUploads, FetchUploadTransport, resumeUpload } from "./uploads/feature.js";
export { reacquireUpload } from "./uploads/resume.js";
export type {
  ReacquiredUpload,
  ReacquiredTransfer,
  UploadApplicationPort,
  UploadFeatureOptions,
  UploadFileIdentity,
  UploadHandle,
  UploadTransport,
  UploadTransportRequest,
  UploadTransportResponse,
  UploadResumeRequest,
} from "./uploads/public.js";
export const uploadsRegistration = registerRuntimeFeature(globalThis, uploadsFeature);

export default uploadsFeature;
