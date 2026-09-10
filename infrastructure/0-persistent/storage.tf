# Create S3 bucket for K3S master backups
resource "minio_s3_bucket" "k3s_backup_bucket" {
  bucket         = local.k3s_backup_bucket_name
  acl            = "private"
  object_locking = false

  # FAU divergence from upstream: upstream ships force_destroy = true with the lifecycle
  # block commented out. Erik chose the safe setting on 8 September 2026.
  #
  # prevent_destroy makes Terraform refuse any plan that would destroy or REPLACE this
  # bucket, including a whole-stage `terraform destroy`. force_destroy = false is the
  # second guard: it stops Terraform deleting a bucket that still has objects in it, in
  # case prevent_destroy is ever lifted.
  #
  # To retire the bucket deliberately, remove the lifecycle block and set
  # force_destroy = true in the same change, then plan and read it before applying.
  force_destroy = false

  lifecycle {
    prevent_destroy = true
  }
}

# Create S3 bucket for DB backups
resource "minio_s3_bucket" "db_backup_bucket" {
  bucket         = local.db_backup_bucket_name
  acl            = "private"
  object_locking = false

  # We need explicitly prevent destroy to avoid accidental deletion of db backups.
  # See the note on k3s_backup_bucket above.
  force_destroy = false

  lifecycle {
    prevent_destroy = true
  }
}
