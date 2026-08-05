export type WorkerErrorCode =
  | "request.invalid_json"
  | "request.trailing_data"
  | "request.duplicate_key"
  | "request.root_not_object"
  | "request.unknown_field"
  | "request.missing_field"
  | "request.wrong_type"
  | "request.unsupported_value"
  | "session.run_failed"
  | "session.invalid_response"
  | "policy.final_url_rejected"
  | "policy.assertion_failed"
  | "evidence.invalid_reference"
  | "evidence.staging_failed"
  | "clock.failed"
  | "clock.invalid_value"
  | "session.finalization_failed";

const messages: Readonly<Record<WorkerErrorCode, string>> = {
  "request.invalid_json": "Browser smoke request is not valid JSON.",
  "request.trailing_data": "Browser smoke request has trailing data.",
  "request.duplicate_key": "Browser smoke request contains a duplicate key.",
  "request.root_not_object": "Browser smoke request root must be an object.",
  "request.unknown_field": "Browser smoke request contains an unknown field.",
  "request.missing_field": "Browser smoke request is missing a required field.",
  "request.wrong_type": "Browser smoke request contains a field with the wrong type.",
  "request.unsupported_value": "Browser smoke request contains an unsupported value.",
  "session.run_failed": "Browser session execution failed.",
  "session.invalid_response": "Browser session returned an invalid response.",
  "policy.final_url_rejected": "Browser session returned an unacceptable final URL.",
  "policy.assertion_failed": "Browser smoke assertion failed.",
  "evidence.invalid_reference": "Host returned an invalid evidence reference.",
  "evidence.staging_failed": "Host failed to stage generated evidence.",
  "clock.failed": "Worker clock failed.",
  "clock.invalid_value": "Worker clock returned an invalid value.",
  "session.finalization_failed": "Browser session finalization failed.",
};

export class BrowserSmokeWorkerError extends Error {
  readonly code: WorkerErrorCode;

  constructor(code: WorkerErrorCode) {
    super(messages[code]);
    this.name = "BrowserSmokeWorkerError";
    this.code = code;
  }
}
