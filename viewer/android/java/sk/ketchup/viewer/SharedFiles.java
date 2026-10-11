package sk.ketchup.viewer;

import android.content.ContentProvider;
import android.content.ContentValues;
import android.database.Cursor;
import android.database.MatrixCursor;
import android.net.Uri;
import android.os.ParcelFileDescriptor;
import android.provider.OpenableColumns;
import java.io.File;
import java.io.FileNotFoundException;

/**
 * Lends the files the Viewer shares (comments for the author) to the app the
 * customer picks in the share sheet, read-only and only through the URI
 * permission the share intent grants. Files live in the cache's shared folder.
 */
public class SharedFiles extends ContentProvider {
    static final String AUTHORITY = "sk.ketchup.viewer.files";

    static File folder(android.content.Context context) {
        return new File(context.getCacheDir(), "shared");
    }

    /** The shared file a URI names; names never leave the shared folder. */
    private File file(Uri uri) throws FileNotFoundException {
        String name = uri.getLastPathSegment();
        if (name == null || name.isEmpty() || name.contains("/") || name.equals("..")) {
            throw new FileNotFoundException(String.valueOf(uri));
        }
        File file = new File(folder(getContext()), name);
        if (!file.isFile()) {
            throw new FileNotFoundException(String.valueOf(uri));
        }
        return file;
    }

    @Override
    public boolean onCreate() {
        return true;
    }

    @Override
    public ParcelFileDescriptor openFile(Uri uri, String mode) throws FileNotFoundException {
        if (!"r".equals(mode)) {
            throw new FileNotFoundException("Shared files are read-only");
        }
        return ParcelFileDescriptor.open(file(uri), ParcelFileDescriptor.MODE_READ_ONLY);
    }

    /** Name and size, which mail apps show on the attachment. */
    @Override
    public Cursor query(Uri uri, String[] projection, String selection, String[] args, String order) {
        File file;
        try {
            file = file(uri);
        } catch (FileNotFoundException missing) {
            return null;
        }
        MatrixCursor cursor = new MatrixCursor(
                new String[] {OpenableColumns.DISPLAY_NAME, OpenableColumns.SIZE});
        cursor.addRow(new Object[] {file.getName(), file.length()});
        return cursor;
    }

    @Override
    public String getType(Uri uri) {
        return "application/octet-stream";
    }

    @Override
    public Uri insert(Uri uri, ContentValues values) {
        throw new UnsupportedOperationException();
    }

    @Override
    public int delete(Uri uri, String selection, String[] args) {
        throw new UnsupportedOperationException();
    }

    @Override
    public int update(Uri uri, ContentValues values, String selection, String[] args) {
        throw new UnsupportedOperationException();
    }
}
