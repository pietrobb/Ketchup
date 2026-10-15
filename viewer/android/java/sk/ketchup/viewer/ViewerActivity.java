package sk.ketchup.viewer;

import android.app.NativeActivity;
import android.content.ClipData;
import android.content.ComponentName;
import android.content.Intent;
import android.database.Cursor;
import android.net.Uri;
import android.os.Bundle;
import android.os.ParcelFileDescriptor;
import android.provider.OpenableColumns;
import java.io.File;
import java.io.FileOutputStream;
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

    /**
     * Offers {@code data} as file {@code name} in the system share sheet;
     * false with {@link #lastError} set when it could not be prepared.
     */
    public boolean share(String name, byte[] data) {
        File folder = SharedFiles.folder(this);
        File file = new File(folder, name);
        if (!file.getParentFile().equals(folder)) {
            lastError = "Not a plain file name: " + name;
            return false;
        }
        try {
            // Earlier shares are done with; only the newest file stays lent out.
            File[] old = folder.listFiles();
            if (old != null) {
                for (File each : old) {
                    each.delete();
                }
            }
            if (!folder.isDirectory() && !folder.mkdirs()) {
                lastError = "Cannot create " + folder;
                return false;
            }
            try (OutputStream out = new FileOutputStream(file)) {
                out.write(data);
            }
        } catch (Throwable error) {
            lastError = error.toString();
            return false;
        }
        Uri uri = new Uri.Builder().scheme("content").authority(SharedFiles.AUTHORITY)
                .appendPath(name).build();
        Intent send = new Intent(Intent.ACTION_SEND)
                .setType("application/octet-stream")
                .putExtra(Intent.EXTRA_STREAM, uri)
                .addFlags(Intent.FLAG_GRANT_READ_URI_PERMISSION);
        send.setClipData(ClipData.newRawUri(name, uri));
        Intent chooser = Intent.createChooser(send, null)
                .putExtra(Intent.EXTRA_EXCLUDE_COMPONENTS,
                        new ComponentName[] {new ComponentName(this, ViewerActivity.class)});
        runOnUiThread(() -> startActivity(chooser));
        return true;
    }

    /** The file name the providing app shows for {@code uri}, or "". */
    public String displayName(String uri) {
        try (Cursor cursor = getContentResolver().query(Uri.parse(uri),
                new String[] {OpenableColumns.DISPLAY_NAME}, null, null, null)) {
            if (cursor != null && cursor.moveToFirst() && !cursor.isNull(0)) {
                return cursor.getString(0);
            }
        } catch (Throwable error) {
            lastError = error.toString();
        }
        return "";
    }

    /**
     * A file descriptor of {@code uri} opened in {@code mode} ("r" to read,
     * "wt" to replace), which native code reads or writes and closes; -1 with
     * {@link #lastError} set. The file's bytes never pass through the Java
     * heap, so a large package cannot run it out of memory.
     */
    public int openFile(String uri, String mode) {
        try {
            ParcelFileDescriptor file = getContentResolver().openFileDescriptor(Uri.parse(uri), mode);
            if (file == null) {
                lastError = "The app holding the file returned no file";
                return -1;
            }
            return file.detachFd();
        } catch (Throwable error) {
            lastError = error.toString();
            return -1;
        }
    }

    /** Why the last open, share or name lookup failed. */
    public String lastError() {
        return lastError;
    }
}
