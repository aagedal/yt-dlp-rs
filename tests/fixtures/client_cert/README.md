Test CA (ca.der) and a CA-signed P-256 client certificate (clientAuth) for --client-certificate tests:
cert.pem, key.pem (PKCS#8), combined.pem (both), encrypted-key.pem (password "secret").
legacy-encrypted-key.pem is key.pem in OpenSSL's legacy encrypted PEM (`openssl ec -aes256`,
password "secret").
