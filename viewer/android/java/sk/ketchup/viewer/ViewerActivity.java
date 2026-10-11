package sk.ketchup.viewer;

import android.app.NativeActivity;
import android.content.Intent;
import android.database.Cursor;
import android.net.Uri;
import android.os.Bundle;
import android.provider.OpenableColumns;
import java.io.ByteArrayOutputStream;
import java.io.InputStream;
import java.io.OutputStream;
import java.util.ArrayDeque;

/**
 * The Viewer's NativeActivity plus the file traffic native code cannot do on
 * its own: files other apps open with the Viewer, the system file picker and
 * the "save as" dialog. Called from Rust through JNI; no method throws.
 */
public class ViewerActivity extends NativeActivity {
    /** Request code of a file another app opened with the Viewer. */
    private static final int OPENED_BY_ANOTHER_APP = 0;

    private final ArrayDeque<String> results = new ArrayDeque<>();
    private volatile String lastError = "";

    @Override
    protected void onCreate(Bundle state) {
        super.onCreate(state);
        if (state == null) {
            received(getIntent());
        }
    }

    @Override
    protected void onNewIntent(Intent intent) {
        super.onNewIntent(intent);
        setIntent(intent);
        received(intent);
    }

    @SuppressWarnings("deprecation") // getParcelableExtra(String, Class) needs API 33.
    private void received(Intent intent) {
        Uri uri = null;
        if (Intent.ACTION_VIEW.equals(intent.getAction())) {
            uri = intent.getData();
        } else if (Intent.ACTION_SEND.equals(intent.getAction())) {
            uri = intent.getParcelableExtra(Intent.EXTRA_STREAM);
        }
        if (uri != null) {
            push(OPENED_BY_ANOTHER_APP, uri);
        }
    }

    @Override
    protected void onActivityResult(int code, int result, Intent data) {
        super.onActivityResult(code, result, data);
        Uri uri = result == RESULT_OK && data != null ? data.getData() : null;
        push(code, uri);
    }

    private synchronized void push(int code, Uri uri) {
        results.add(code + "\n" + (uri == null ? "" : uri.toString()));
    }

    /** The next "code\nuri" the native side has not taken yet, or null; an empty uri is a cancelled dialog. */
    public synchronized String takeResult() {
        return results.poll();
    }

    /** Lets the customer pick a file; the result arrives under {@code code}. */
    public void pickFile(int code) {
        runOnUiThread(() -> {
            Intent intent = new Intent(Intent.ACTION_OPEN_DOCUMENT)
                    .addCategory(Intent.CATEGORY_OPENABLE)
                    .setType("*/*");
            startActivityForResult(intent, code);
        });
    }

    /** Lets the customer choose where {@code name} is saved; the result arrives under {@code code}. */
    public void createFile(int code, String name) {
        runOnUiThread(() -> {
            Intent intent = new Intent(Intent.ACTION_CREATE_DOCUMENT)
                    .addCategory(Intent.CATEGORY_OPENABLE)
                    .setType("application/octet-stream")
                    .putExtra(Intent.EXTRA_TITLE, name);
            startActivityForResult(intent, code);
        });
    }

    /** The file name the providing app shows for {@code uri}, or "". */
    public String displayName(String uri) {
        try (Cursor cursor = getContentResolver().query(Uri.parse(uri),
                new String[] {OpenableColumns.DISPLAY_NAME}, null, null, null)) {
            if (cursor != null && cursor.moveToFirst() && !cursor.isNull(0)) {
                return cursor.getString(0);
            }
        } catch (Exception error) {
            lastError = error.toString();
        }
        return "";
    }

    /** The whole file, or null with {@link #lastError} set. */
    public byte[] read(String uri) {
        try (InputStream in = getContentResolver().openInputStream(Uri.parse(uri))) {
            if (in == null) {
                lastError = "The app holding the file returned no data";
                return null;
            }
            ByteArrayOutputStream bytes = new ByteArrayOutputStream();
            byte[] buffer = new byte[64 * 1024];
            for (int read; (read = in.read(buffer)) > 0; ) {
                bytes.write(buffer, 0, read);
            }
            return bytes.toByteArray();
        } catch (Exception error) {
            lastError = error.toString();
            return null;
        }
    }

    /** Replaces the file with {@code data}; false with {@link #lastError} set. */
    public boolean write(String uri, byte[] data) {
        try (OutputStream out = getContentResolver().openOutputStream(Uri.parse(uri), "wt")) {
            if (out == null) {
                lastError = "The app holding the file refused writing";
                return false;
            }
            out.write(data);
            return true;
        } catch (Exception error) {
            lastError = error.toString();
            return false;
        }
    }

    /** Why the last read, write or name lookup failed. */
    public String lastError() {
        return lastError;
    }
}
