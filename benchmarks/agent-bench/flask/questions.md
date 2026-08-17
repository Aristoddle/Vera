# Agent-Level Vera Benchmark — Flask question set

For every subquestion, trace the current implementation across files and cite decisive evidence using `path:line` or `path:start-line-end-line` references.

## Question 1
Trace an ordinary WSGI request from the Flask callable through context creation, context push, URL matching, request dispatch, response finalization, and context pop.

1. Which context object is created and made active, and which context-dependent objects become available at each stage?
2. Place session opening, app-context push signaling, URL matching, request-start signaling, and request-finished signaling in their actual order.
3. Explain what changes when the same context is pushed more than once, and identify which operation is responsible for passing an exception into cleanup.

## Question 2
Assume an app has request teardown callbacks, blueprint request teardown callbacks, app-context teardown callbacks, and receivers for the corresponding teardown signals. Trace cleanup for both a successful request and an unhandled exception.

1. Give the exact ordering among blueprint callbacks, app callbacks, the request teardown signal, request closing, app-context callbacks, the app-context teardown signal, context-variable reset, and the popped signal.
2. Explain how callback registration order affects request and app-context teardown, including nested blueprint names.
3. Explain what happens when multiple teardown callbacks or teardown signal receivers raise exceptions, and what exception value each teardown callback receives for handled versus unhandled request errors.

## Question 3
Trace how a Flask application's initial configuration is constructed and how repeated configuration updates determine the final value of a shared uppercase key.

1. Identify the initial defaults, the source of the initial `DEBUG` value, and how `instance_relative_config` changes the root used for relative files.
2. Given any sequence of `from_object`, `from_pyfile`, `from_file`, `from_mapping`, `from_envvar`, and `from_prefixed_env` calls, state which update wins for a key and which keys are ignored.
3. Explain whether Flask automatically loads any external configuration file during application construction, and support the conclusion from the constructor path.

## Question 4
Compare the configuration loaders and their failure and conversion behavior.

1. Trace `from_object` and `from_pyfile`, including import-string handling, uppercase filtering, relative path resolution, execution of a Python file, and silent missing-file behavior.
2. Trace `from_file`, `from_mapping`, and `from_envvar`, including loader invocation, keyword precedence, relative paths, and the difference between an unset environment variable and a missing file.
3. Trace `from_prefixed_env`: key ordering, prefix stripping, JSON conversion failures, and nested keys separated by double underscores.

## Question 5
Compare route registration directly on an app with route registration on a blueprint that is later registered on the app.

1. Follow the route decorator and URL-rule path in both cases, and state when a blueprint route becomes an application URL rule and view-function mapping.
2. Derive the final URL rule, endpoint name, and URL defaults for a blueprint route with a registration `url_prefix`, a custom registration name, and nested blueprints.
3. Explain how blueprint registration handles deferred callbacks, repeated registration, and endpoint collisions, and contrast that with direct app registration.

## Question 6
Trace callback and error-handler scope across an application, a blueprint, and a nested blueprint.

1. Distinguish a blueprint's local `before_request`, `after_request`, `teardown_request`, and `errorhandler` registrations from its `before_app_request`, `after_app_request`, `teardown_app_request`, and `app_errorhandler` registrations.
2. For a request handled by a nested blueprint, derive the callback order for preprocessing, after processing, and teardown, including app-wide callbacks.
3. For an HTTP exception with both code and class handlers at multiple scopes, derive the handler lookup order and explain how the request's dotted blueprint name supplies the scopes that are searched.

## Question 7
Trace the default signed-cookie session's serialization and deserialization path for a normal cookie, a tampered cookie, and an expired cookie.

1. Explain how the signing serializer is constructed, including the current secret key, fallback keys, salt, key derivation, digest, and tagged JSON serializer.
2. Explain how session data is serialized into a cookie and restored, including the non-standard Python value types supported by the tagged serializer.
3. Compare the behavior for a bad signature and an expired timestamp, including the resulting session object and whether either condition is surfaced as an application error.

## Question 8
Trace session behavior when the application has no usable secret key and when a usable session is saved at the end of a request.

1. Explain how `open_session` returning `None` is converted into a null session, which operations the null session permits, and whether it is saved.
2. Explain when session access adds a `Vary` header, when an empty modified session deletes its cookie, and when a non-empty session receives a new cookie.
3. Explain how permanence, expiration, refresh configuration, and session modification affect the save decision, and place session saving relative to after-request callbacks and request teardown.

## Question 9
Trace Flask's signal definitions and every default dispatch point relevant to requests, contexts, templates, exceptions, and flashing.

1. Identify which signals are defined by Flask and where request, app-context, template, exception, and flash signals are emitted.
2. Derive the ordering of request-started, request-finished, request-tearing-down, app-context-pushed, app-context-tearing-down, and app-context-popped relative to their surrounding callbacks and cleanup operations.
3. Determine whether the current Flask source installs any default signal subscribers, and explain what a receiver must do to observe the signals and what arguments it receives.

## Question 10
Trace the error and proxy behavior for an `HTTPException`, a generic exception, and code running with different debug or testing settings.

1. Compare the paths for an `HTTPException` and a generic `Exception` when no matching handler exists, including routing exceptions, `got_request_exception`, the 500 wrapper, and finalization.
2. Explain how `PROPAGATE_EXCEPTIONS`, `TESTING`, `DEBUG`, `TRAP_HTTP_EXCEPTIONS`, and `TRAP_BAD_REQUEST_ERRORS` change propagation or handler selection.
3. Trace how `current_app` and `request` resolve through their proxies in an app-only context, a request context, and no active context, including the failures a caller observes outside the required context.
