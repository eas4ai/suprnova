import {
  uploadFileIdentitiesEqual,
  uploadFileIdentity,
  validateTransferGrant,
  validateNextChunkIndex,
  validateUploadedBytes,
  validateUploadField,
  validateUploadHandle,
  validateUploadRevision,
  type ReacquiredTransfer,
  type UploadApplicationPort,
  type UploadHandle,
} from "./types.js";

export async function reacquireUpload(
  application: UploadApplicationPort | undefined,
  request: Readonly<{ field: string; file: File; handle: UploadHandle }>,
): Promise<ReacquiredTransfer> {
  if (application === undefined) throw new Error("upload_reacquire_unavailable");
  validateUploadField(request.field);
  validateUploadHandle(request.handle);
  const fileIdentity = uploadFileIdentity(request.file);
  const reacquired = await application.reacquire(
    Object.freeze({ field: request.field, fileIdentity, handle: request.handle }),
  );
  if (!uploadFileIdentitiesEqual(fileIdentity, reacquired.fileIdentity)) {
    throw new Error("upload_reacquire_identity_mismatch");
  }
  validateTransferGrant(reacquired.grant);
  validateUploadRevision(reacquired.revision);
  validateUploadedBytes(reacquired.uploadedBytes, fileIdentity.size);
  validateNextChunkIndex(reacquired.nextChunkIndex);
  if (reacquired.nextChunkIndex > reacquired.uploadedBytes) {
    throw new Error("upload_next_chunk_index_invalid");
  }
  const state: unknown = reacquired.state;
  if (state !== "queued" && state !== "transferring" && state !== "verifying") {
    throw new Error("upload_reacquire_state_invalid");
  }
  return Object.freeze({
    file: request.file,
    fileIdentity,
    grant: reacquired.grant,
    handle: request.handle,
    nextChunkIndex: reacquired.nextChunkIndex,
    revision: reacquired.revision,
    state,
    uploadedBytes: reacquired.uploadedBytes,
  });
}
