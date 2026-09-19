import {
  type ActionFunctionArgs,
  Form,
  Link,
  useActionData,
  useNavigation,
} from "react-router-dom";
import { api, errorMessage } from "../api/client";

interface Result {
  error?: string;
  done?: boolean;
}

export async function action({ request }: ActionFunctionArgs): Promise<Result> {
  const form = await request.formData();
  try {
    await api.register(
      String(form.get("email")),
      String(form.get("password")),
      String(form.get("name")),
    );
  } catch (err) {
    return { error: errorMessage(err, "Registration failed") };
  }
  return { done: true };
}

function RegisterPage() {
  const data = useActionData<Result>();
  const busy = useNavigation().state === "submitting";

  if (data?.done) {
    return (
      <div className="auth-card card">
        <h1>Account created</h1>
        <p className="muted">
          New accounts start with the <strong>staff</strong> role. A manager (or
          an administrator) grants management access.
        </p>
        <Link to="/login" className="btn btn-primary">
          Continue to sign in
        </Link>
      </div>
    );
  }

  return (
    <div className="auth-card card">
      <h1>Register</h1>
      <Form method="post">
        <label>
          Name
          <input name="name" required />
        </label>
        <label>
          Email
          <input type="email" name="email" autoComplete="username" required />
        </label>
        <label>
          Password
          <input
            type="password"
            name="password"
            autoComplete="new-password"
            required
          />
        </label>
        {data?.error && <div className="alert">{data.error}</div>}
        <button type="submit" className="btn btn-primary" disabled={busy}>
          {busy ? "Creating…" : "Create account"}
        </button>
      </Form>
      <p className="muted">
        Already have an account? <Link to="/login">Sign in</Link>
      </p>
    </div>
  );
}

export { RegisterPage as Component };
