package io.github.francescodibella.repohub;

import android.content.Context;
import android.content.SharedPreferences;
import android.security.keystore.KeyGenParameterSpec;
import android.security.keystore.KeyProperties;
import android.util.Base64;

import java.nio.charset.StandardCharsets;
import java.security.KeyStore;

import javax.crypto.Cipher;
import javax.crypto.KeyGenerator;
import javax.crypto.SecretKey;
import javax.crypto.spec.GCMParameterSpec;

/**
 * The account, in the app's private preferences. The token is encrypted (AES-256-GCM) with a
 * key that lives in the Android Keystore and can't be read out of it, so a copy of the app's
 * files alone doesn't give the token away. Backups leave these files out (see the manifest).
 */
final class AccountStore implements Backend.Account {
    private static final String KEY_ALIAS = "repohub-account";
    private final SharedPreferences prefs;

    AccountStore(Context context) {
        prefs = context.getSharedPreferences("account", Context.MODE_PRIVATE);
    }

    private static SecretKey key() throws Exception {
        KeyStore store = KeyStore.getInstance("AndroidKeyStore");
        store.load(null);
        if (store.getKey(KEY_ALIAS, null) instanceof SecretKey) {
            return (SecretKey) store.getKey(KEY_ALIAS, null);
        }
        KeyGenerator gen = KeyGenerator.getInstance(KeyProperties.KEY_ALGORITHM_AES, "AndroidKeyStore");
        gen.init(new KeyGenParameterSpec.Builder(KEY_ALIAS, KeyProperties.PURPOSE_ENCRYPT | KeyProperties.PURPOSE_DECRYPT)
            .setBlockModes(KeyProperties.BLOCK_MODE_GCM)
            .setEncryptionPaddings(KeyProperties.ENCRYPTION_PADDING_NONE)
            .setKeySize(256)
            .build());
        return gen.generateKey();
    }

    @Override
    public synchronized String user() {
        return prefs.getString("user", null);
    }

    @Override
    public synchronized String token() {
        String iv = prefs.getString("token_iv", null);
        String sealed = prefs.getString("token", null);
        if (iv == null || sealed == null) {
            return null;
        }
        try {
            Cipher c = Cipher.getInstance("AES/GCM/NoPadding");
            c.init(Cipher.DECRYPT_MODE, key(), new GCMParameterSpec(128, Base64.decode(iv, Base64.NO_WRAP)));
            return new String(c.doFinal(Base64.decode(sealed, Base64.NO_WRAP)), StandardCharsets.UTF_8);
        } catch (Exception e) {
            // The key is gone (the app's data was restored on another device): no token.
            return null;
        }
    }

    @Override
    public synchronized void save(String user, String token) throws Exception {
        SharedPreferences.Editor e = prefs.edit();
        if (user == null) {
            e.remove("user");
        } else {
            e.putString("user", user);
        }
        if (token == null) {
            e.remove("token").remove("token_iv");
        } else {
            Cipher c = Cipher.getInstance("AES/GCM/NoPadding");
            c.init(Cipher.ENCRYPT_MODE, key());
            byte[] sealed = c.doFinal(token.getBytes(StandardCharsets.UTF_8));
            e.putString("token", Base64.encodeToString(sealed, Base64.NO_WRAP));
            e.putString("token_iv", Base64.encodeToString(c.getIV(), Base64.NO_WRAP));
        }
        if (!e.commit()) {
            throw new java.io.IOException("the phone's storage refused the change");
        }
    }
}
