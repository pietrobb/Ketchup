package sk.ketchup.viewer;

import android.app.NativeActivity;
import android.content.ClipData;
import android.content.ComponentName;
import android.content.Intent;
import android.content.UriPermission;
import android.database.Cursor;
import android.net.Uri;
import android.os.Bundle;
import android.os.ParcelFileDescriptor;
import android.provider.OpenableColumns;
import java.io.File;
import java.io.FileOutputStream;
import java.io.OutputStream;
import java.util.ArrayDeque;
import java.util.Arrays;
import java.util.HashMap;
import java.util.HashSet;

/**
 * The Viewer's NativeActivity plus the file traffic native code cannot do on
 * its own: files other apps open with the Viewer, the system file picker and
 * the "save as" dialog. Called from Rust through JNI; no method throws.
 */
public class ViewerActivity extends NativeActivity {
    /** Request code of a file another app opened with the Viewer. */
    private static final int OPENED_BY_ANOTHER_APP = 0;

    private final ArrayDeque<String> results = new ArrayDeque<>();
    /** The grants a dialog offered to keep across starts, by uri; taken only while a save needs them. */
    private final HashMap<String, Integer> offered = new HashMap<>();
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
        if (uri != null && (data.getFlags() & Intent.FLAG_GRANT_PERSISTABLE_URI_PERMISSION) != 0) {
            synchronized (this) {
                offered.put(uri.toString(), data.getFlags()
                        & (Intent.FLAG_GRANT_READ_URI_PERMISSION | Intent.FLAG_GRANT_WRITE_URI_PERMISSION));
            }
        }
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
     * "wt" to replace), which native code reads or writes and closes; with
     * {@link #lastError} set, -2 when the file is gone, -3 when the Viewer may
     * no longer open it and -1 for any other failure. The file's bytes never
     * pass through the Java heap, so a large package cannot run it out of
     * memory.
     */
    public int openFile(String uri, String mode) {
        try {
            ParcelFileDescriptor file = getContentResolver().openFileDescriptor(Uri.parse(uri), mode);
            if (file == null) {
                lastError = "The app holding the file returned no file";
                return -1;
            }
            return file.detachFd();
        } catch (java.io.FileNotFoundException error) {
            lastError = error.toString();
            // A provider that still lists the document only cannot reach it
            // now (a cloud offline): not gone (review 2026-10-24 VW-21).
            return listed(uri) ? -1 : -2;
        } catch (SecurityException error) {
            lastError = error.toString();
            return -3;
        } catch (Throwable error) {
            lastError = error.toString();
            return -1;
        }
    }

    /** Whether the provider holding {@code uri} answers a query with its row. */
    private boolean listed(String uri) {
        try (Cursor cursor = getContentResolver().query(Uri.parse(uri),
                new String[] {OpenableColumns.DISPLAY_NAME}, null, null, null)) {
            return cursor != null && cursor.moveToFirst();
        } catch (Throwable error) {
            return false;
        }
    }

    /**
     * Keeps the grant to write {@code uri} across starts while a save to it
     * runs: a save Android cuts off by ending the app is undone at the next
     * start, which writes the document again. False with {@link #lastError}
     * set when it could not be kept; a uri no dialog offered to keep has
     * nothing to keep.
     */
    public boolean holdGrant(String uri) {
        Integer flags;
        synchronized (this) {
            flags = offered.get(uri);
        }
        if (flags == null) {
            return true;
        }
        try {
            getContentResolver().takePersistableUriPermission(Uri.parse(uri), flags);
            return true;
        } catch (Throwable error) {
            lastError = error.toString();
            return false;
        }
    }

    /**
     * Gives back every grant kept across starts except those for the
     * {@code keep} uris, one per line: Android allows an app only so many.
     */
    public void releaseGrantsExcept(String keep) {
        HashSet<String> kept = new HashSet<>(Arrays.asList(keep.split("\n")));
        try {
            for (UriPermission grant : getContentResolver().getPersistedUriPermissions()) {
                if (kept.contains(grant.getUri().toString())) {
                    continue;
                }
                int flags = (grant.isReadPermission() ? Intent.FLAG_GRANT_READ_URI_PERMISSION : 0)
                        | (grant.isWritePermission() ? Intent.FLAG_GRANT_WRITE_URI_PERMISSION : 0);
                getContentResolver().releasePersistableUriPermission(grant.getUri(), flags);
            }
        } catch (Throwable error) {
            lastError = error.toString();
        }
    }

    /** Why the last open, share or name lookup failed. */
    public String lastError() {
        return lastError;
    }
}
